//! specs/0087 — the MCP tool router behind `nixon --mcp`: three read-only tools that are
//! thin wrappers over [`super::queries`].
//!
//! Every call, in order: the on/off switch (re-read from disk each time, so turning access
//! off takes effect on the next call), the meetings DB existing, its schema being current,
//! then the query. Every call is written to the access log, refused or not. Refusals are
//! plain text results (not JSON-RPC errors) so the agent shows them to the user; bad
//! arguments and failures are text results marked `isError`.

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler, ServiceExt};
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

use super::entry::{open_meetings_pool, schema_is_current, McpArgs, DB_FILE};
use super::log::{AccessEntry, AccessLog};
use super::queries::{self, GetArgs, ListArgs, SearchArgs, ToolOutput};
use super::settings;

pub const OFF_TEXT: &str = "Nixon's agent access is turned off. Turn it on in Nixon › Settings.";
pub const NEEDS_UPDATE_TEXT: &str = "Open Nixon once to finish updating, then try again.";
pub const NO_DB_TEXT: &str =
    "Nixon has no meetings here yet. Open Nixon and record a meeting first.";

const LOG_RETENTION_DAYS: i64 = 90;

const SEARCH_DESC: &str = "Search all of the user's recorded meetings (transcripts, summaries, notes) for a topic, decision or piece of feedback. Use this first when you don't know which meeting something was in. Results include who said it, when, and a nixon:// link to that exact moment — cite those links when you use what you found.";
const LIST_DESC: &str = "List the user's meetings, newest first, filtered by date range, a person who attended or spoke, or words in the title. Use it to narrow down before reading a meeting.";
const GET_DESC: &str = "Read one meeting: summary, the user's notes, action items, and the transcript with speaker names, timestamps and nixon:// links per line. Pass from_seconds/to_seconds to read only the stretch around a search hit. Long transcripts come in pages; follow the cursor to read on.";

/// Serves until the client closes stdin.
pub async fn serve(args: McpArgs) -> Result<()> {
    let server = NixonServer::new(args);
    match server.log().await {
        Ok(log) => {
            if let Err(e) = log.prune_older_than_days(LOG_RETENTION_DAYS).await {
                eprintln!("nixon --mcp: could not prune the access log: {e:#}");
            }
        }
        Err(e) => eprintln!("nixon --mcp: could not open the access log: {e:#}"),
    }
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

struct Inner {
    data_dir: PathBuf,
    scheme: String,
    /// Opened on first use and kept for the life of the process.
    pool: OnceCell<SqlitePool>,
    log: OnceCell<AccessLog>,
}

#[derive(Clone)]
pub struct NixonServer {
    inner: Arc<Inner>,
    tool_router: ToolRouter<Self>,
}

enum Call {
    Search(SearchArgs),
    List(ListArgs),
    Get(GetArgs),
}

#[tool_router]
impl NixonServer {
    fn new(args: McpArgs) -> Self {
        Self {
            inner: Arc::new(Inner {
                data_dir: args.data_dir,
                scheme: args.link_scheme,
                pool: OnceCell::new(),
                log: OnceCell::new(),
            }),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = SEARCH_DESC)]
    async fn search_meetings(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<SearchArgs>,
    ) -> CallToolResult {
        self.handle(&peer, "search_meetings", json(&args), Call::Search(args))
            .await
    }

    #[tool(description = LIST_DESC)]
    async fn list_meetings(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<ListArgs>,
    ) -> CallToolResult {
        self.handle(&peer, "list_meetings", json(&args), Call::List(args))
            .await
    }

    #[tool(description = GET_DESC)]
    async fn get_meeting(
        &self,
        peer: Peer<RoleServer>,
        Parameters(args): Parameters<GetArgs>,
    ) -> CallToolResult {
        self.handle(&peer, "get_meeting", json(&args), Call::Get(args))
            .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NixonServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new(self.inner.scheme.clone(), env!("CARGO_PKG_VERSION")),
        )
    }
}

fn json<T: Serialize>(args: &T) -> serde_json::Value {
    serde_json::to_value(args).unwrap_or(serde_json::Value::Null)
}

impl NixonServer {
    async fn log(&self) -> Result<&AccessLog> {
        self.inner
            .log
            .get_or_try_init(|| AccessLog::open(&self.inner.data_dir))
            .await
    }

    async fn handle(
        &self,
        peer: &Peer<RoleServer>,
        tool: &str,
        args: serde_json::Value,
        call: Call,
    ) -> CallToolResult {
        let (out, outcome) = self.gate_and_run(call).await;
        let entry = AccessEntry {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            client: peer.peer_info().map(|i| i.client_info.name.clone()),
            tool: tool.to_string(),
            args,
            meeting_ids: out.meeting_ids.clone(),
            outcome: outcome.to_string(),
        };
        match self.log().await {
            Ok(log) => {
                if let Err(e) = log.record(&entry).await {
                    eprintln!("nixon --mcp: could not write the access log: {e:#}");
                }
            }
            Err(e) => eprintln!("nixon --mcp: could not open the access log: {e:#}"),
        }
        let content = vec![ContentBlock::text(out.text)];
        if out.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        }
    }

    /// The per-call gate, then the query. Returns the output and its log outcome.
    async fn gate_and_run(&self, call: Call) -> (ToolOutput, &'static str) {
        let inner = &self.inner;
        if !settings::load(&inner.data_dir).enabled {
            return (ToolOutput::plain(OFF_TEXT), "off");
        }
        if !inner.data_dir.join(DB_FILE).is_file() {
            return (ToolOutput::plain(NO_DB_TEXT), "no_db");
        }
        let pool = match inner
            .pool
            .get_or_try_init(|| open_meetings_pool(&inner.data_dir))
            .await
        {
            Ok(p) => p,
            Err(e) => return (failure(e), "error"),
        };
        if !schema_is_current(pool).await {
            return (ToolOutput::plain(NEEDS_UPDATE_TEXT), "needs_update");
        }
        let scheme = inner.scheme.as_str();
        let result = match call {
            Call::Search(a) => queries::search_meetings(pool, scheme, a).await,
            Call::List(a) => queries::list_meetings(pool, scheme, a).await,
            Call::Get(a) => queries::get_meeting(pool, scheme, a).await,
        };
        match result {
            Ok(out) if out.is_error => (out, "error"),
            Ok(out) => (out, "ok"),
            Err(e) => (failure(e), "error"),
        }
    }
}

fn failure(e: anyhow::Error) -> ToolOutput {
    ToolOutput {
        text: format!("{e:#}"),
        is_error: true,
        ..ToolOutput::default()
    }
}
