//! Google OAuth for the Calendar provider (specs/0032 task 1, ADR-0010).
//!
//! Flow shape (authorization-code + **PKCE S256** + `state`, desktop-app
//! loopback redirect):
//!
//! 1. [`start_connect_flow`] binds an ephemeral loopback listener
//!    (`127.0.0.1:0`) and returns a [`PendingConnect`] carrying the Google
//!    consent URL. **The caller opens `auth_url` in the browser** via the
//!    app's existing allowlisted opener (`api::open_external_url`) — this
//!    module never launches anything itself.
//! 2. [`PendingConnect::finish`] waits (max [`CONNECT_TIMEOUT`], then a clean
//!    "cancelled" error) for Google's redirect, validates `state`, exchanges
//!    the code (PKCE verifier attached), and persists the **refresh token**
//!    in the Keychain under [`secrets::GCAL_REFRESH_TOKEN_ACCOUNT`].
//!    Per ADR-0010 there is **no DB/sentinel fallback**: a failed Keychain
//!    write aborts the connect and nothing is persisted anywhere.
//! 3. [`get_access_token`] serves later API calls from the **in-memory**
//!    access-token cache (module state — access tokens never touch disk),
//!    refreshing through the stored refresh token when expired. A refresh
//!    rejected with `invalid_grant` (user revoked access / token expired) is
//!    detectable via [`is_invalid_grant`] so the command layer can emit the
//!    `google-calendar-auth-required` event.
//! 4. [`revoke_and_clear`] best-effort revokes at Google, then deletes the
//!    Keychain item and the in-memory cache (disconnect path).
//!
//! Scopes are exactly the narrowest read-only pair sanctioned by ADR-0010
//! ([`SCOPES`]). **Never log token material — not even prefixes.** Errors from
//! this module carry user-actionable text and no secrets.
//!
//! All entry points take the [`SecretStore`] explicitly (callers pass
//! `crate::secrets::store()`; a `None` store means the Keychain is unavailable
//! and connect/refresh must fail — see ADR-0010) so unit tests run against
//! `MemoryStore` with mocked HTTP endpoints, never the real Keychain/network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use oauth2::basic::{BasicClient, BasicErrorResponseType};
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, RequestTokenError, Scope, TokenResponse, TokenUrl,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::secrets::{self, SecretStore};

// ---------------------------------------------------------------------------
// Endpoints / constants
// ---------------------------------------------------------------------------

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

/// The read-only Google scopes Nixon requests (ADR-0010 + its best-effort
/// amendment). The first pair is the narrowest granular combination that
/// supports "list my calendars, read events with attendees". The last two are
/// the amendment's **best-effort** enrichment scopes — requested at consent but
/// used only when the account/org actually grant access at runtime (a granted
/// scope is NOT access: org policy still returns `403 PERMISSION_DENIED`, and we
/// degrade silently to the calendar-only behavior, zero extra egress):
///   * `cloud-identity.groups.readonly` — flatten distribution-list invites into
///     their individual members (Cloud Identity group-member listing, WS3).
///   * `directory.readonly` — attendee directory profiles/photos (a sibling
///     workstream builds the actual People-API fetch; the scope rides along
///     here so that agent needn't touch this file).
///
/// Anything broader still requires a fresh ADR-0010 amendment.
pub const SCOPES: [&str; 4] = [
    "https://www.googleapis.com/auth/calendar.events.readonly",
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    "https://www.googleapis.com/auth/cloud-identity.groups.readonly",
    "https://www.googleapis.com/auth/directory.readonly",
];

/// How long [`PendingConnect::finish`] waits for the browser redirect before
/// giving up with a clean "cancelled" error (spec 0032 failure-modes table).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Refresh this many seconds *before* the access token actually expires, so a
/// token handed to a sync pass can't die mid-request.
const EXPIRY_SLACK_SECS: i64 = 60;

/// Fallback lifetime when Google omits `expires_in` (it never does in
/// practice; Google access tokens live ~3600s).
const DEFAULT_EXPIRY_SECS: u64 = 3600;

// ---------------------------------------------------------------------------
// Error markers (so the command layer can route without string-matching)
// ---------------------------------------------------------------------------

/// Distinct auth failure classes the caller must react to specifically. These
/// ride inside `anyhow::Error`; test with [`is_invalid_grant`] /
/// [`is_cancelled`] (or `err.downcast_ref::<AuthError>()`).
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The refresh token was revoked or expired (`invalid_grant`). The caller
    /// should mark the account disconnected and emit
    /// `google-calendar-auth-required` (spec 0032 failure-modes table).
    #[error(
        "Google Calendar access was revoked or has expired. Reconnect your Google account in Settings."
    )]
    InvalidGrant,
    /// The user abandoned or denied the consent screen (timeout or
    /// `error=access_denied` on the redirect). No state was written.
    #[error("Google Calendar connection cancelled — the sign-in wasn't completed.")]
    Cancelled,
}

/// True when `err` is the revoked/expired-grant marker ([`AuthError::InvalidGrant`]).
pub fn is_invalid_grant(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<AuthError>(),
        Some(AuthError::InvalidGrant)
    )
}

/// True when `err` is the user-abandoned/denied marker ([`AuthError::Cancelled`]).
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    matches!(err.downcast_ref::<AuthError>(), Some(AuthError::Cancelled))
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A live access token. The refresh token is deliberately **not** part of this
/// type — it goes straight to the Keychain and is never handed around.
pub struct TokenBundle {
    /// Bearer token for `www.googleapis.com` calls. In-memory only.
    pub access_token: String,
    /// Instant after which the token must not be used (UTC).
    pub expires_at: DateTime<Utc>,
}

// Manual Debug so an accidental `{:?}` in a log line can never leak the token.
impl std::fmt::Debug for TokenBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBundle")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// An in-flight connect attempt: the loopback listener is already bound and
/// `auth_url` is ready to be opened in the user's browser (by the **caller**,
/// via the allowlisted `api::open_external_url` path). Dropping this without
/// calling [`finish`](Self::finish) abandons the attempt cleanly (the listener
/// closes; nothing was persisted).
pub struct PendingConnect {
    /// The Google consent URL (PKCE challenge + state + scopes baked in).
    pub auth_url: String,
    listener: TcpListener,
    client: BasicClient,
    expected_state: CsrfToken,
    pkce_verifier: PkceCodeVerifier,
    timeout: Duration,
}

// ---------------------------------------------------------------------------
// Connect flow
// ---------------------------------------------------------------------------

/// Begin the browser connect flow: bind the loopback redirect listener and
/// build the consent URL. Fails when the build has no baked-in client id
/// ([`super::is_configured`] is false) or no loopback port could be bound.
///
/// The caller opens [`PendingConnect::auth_url`] externally, then awaits
/// [`PendingConnect::finish`] for the redirect + token exchange.
pub async fn start_connect_flow() -> Result<PendingConnect> {
    // Bind port 0 → the OS picks a free ephemeral port; a stale/occupied port
    // can't happen by construction (spec 0032 failure-modes table).
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("could not open a local port for the Google sign-in redirect")?;
    let port = listener
        .local_addr()
        .context("loopback listener has no local address")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let client_id = super::client_id().ok_or_else(|| {
        anyhow!(
            "Google Calendar is not configured in this build \
             (NIXON_GOOGLE_CLIENT_ID was not set at compile time)"
        )
    })?;
    let client = build_client(
        client_id,
        super::client_secret(),
        TOKEN_ENDPOINT,
        &redirect_uri,
    )?;
    let (auth_url, expected_state, pkce_verifier) = build_auth_request(&client);

    Ok(PendingConnect {
        auth_url,
        listener,
        client,
        expected_state,
        pkce_verifier,
        timeout: CONNECT_TIMEOUT,
    })
}

impl PendingConnect {
    /// Wait for Google's loopback redirect, exchange the code, and persist the
    /// refresh token via `store` (Keychain in production — pass
    /// `crate::secrets::store()`; if that is `None` the connect must fail
    /// upstream, per ADR-0010's "no fallback for OAuth tokens").
    ///
    /// On success the access token is also cached in memory, so an immediate
    /// [`get_access_token`] won't re-hit the network. Every failure leg
    /// returns a user-actionable error and persists **nothing**; abandonment
    /// (timeout / consent denied) is the [`AuthError::Cancelled`] marker.
    pub async fn finish(self, store: &dyn SecretStore) -> Result<TokenBundle> {
        let code =
            wait_for_redirect(self.listener, self.expected_state.secret(), self.timeout).await?;

        let token = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(self.pkce_verifier)
            .request_async(async_http_client)
            .await
            .map_err(|e| {
                anyhow!(
                    "Google rejected the sign-in token exchange: {e}. Please try connecting again."
                )
            })?;

        // Google only returns a refresh token when the consent screen actually
        // ran (we force it with prompt=consent + access_type=offline). Without
        // one the connection couldn't survive an hour — fail loudly instead.
        let refresh = token.refresh_token().ok_or_else(|| {
            anyhow!(
                "Google did not return a long-lived token. Remove Nixon's access at \
                 https://myaccount.google.com/permissions and try connecting again."
            )
        })?;

        // ADR-0010: Keychain or nothing. A failed write aborts the connect;
        // the access token below is memory-only, so nothing was persisted.
        persist_refresh_token(store, refresh.secret())?;

        let bundle = TokenBundle {
            access_token: token.access_token().secret().clone(),
            expires_at: expires_at_from_now(token.expires_in()),
        };
        cache_access(&bundle);
        Ok(bundle)
    }
}

// ---------------------------------------------------------------------------
// Access-token cache + refresh
// ---------------------------------------------------------------------------

/// Return a valid access token: the in-memory cached one when it has more than
/// [`EXPIRY_SLACK_SECS`] left, otherwise refresh through the Keychain-stored
/// refresh token (pass `crate::secrets::store()` as `store`).
///
/// Error routing for callers: [`is_invalid_grant`] ⇒ the grant was revoked or
/// expired — mark disconnected and emit `google-calendar-auth-required`; any
/// other error is transient (offline, quota…) and safe to retry later.
pub async fn get_access_token(store: &dyn SecretStore) -> Result<String> {
    if let Some(token) = cached_access_token() {
        return Ok(token);
    }
    let client_id = super::client_id().ok_or_else(|| {
        anyhow!(
            "Google Calendar is not configured in this build \
             (NIXON_GOOGLE_CLIENT_ID was not set at compile time)"
        )
    })?;
    // The redirect URI is unused on the refresh grant but the client requires
    // one; any well-formed loopback value is fine.
    let client = build_client(
        client_id,
        super::client_secret(),
        TOKEN_ENDPOINT,
        "http://127.0.0.1",
    )?;
    let bundle = refresh_with_client(&client, store).await?;
    Ok(bundle.access_token)
}

/// Best-effort revoke at Google, then delete the Keychain refresh token and
/// drop the in-memory access token (the disconnect path). Revocation failures
/// are logged and swallowed — local cleanup must always proceed — but a failed
/// Keychain **delete** is surfaced (the user needs to know a token is stuck).
pub async fn revoke_and_clear(store: &dyn SecretStore) -> Result<()> {
    revoke_and_clear_at(store, REVOKE_ENDPOINT).await
}

async fn revoke_and_clear_at(store: &dyn SecretStore, revoke_url: &str) -> Result<()> {
    // Read (best-effort) the token to revoke. An unreadable Keychain must not
    // block disconnect — the delete below is the part that matters locally.
    if let Ok(Some(token)) = store.get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT) {
        if !token.is_empty() {
            let result = reqwest::Client::new()
                .post(revoke_url)
                .timeout(Duration::from_secs(10))
                .form(&[("token", token.as_str())])
                .send()
                .await;
            if let Err(e) = result {
                // `e` carries the URL at most — never the form body/token.
                log::warn!("google oauth: best-effort token revoke failed: {e}");
            }
        }
    }
    store
        .delete(secrets::GCAL_REFRESH_TOKEN_ACCOUNT)
        .context("could not remove the stored Google Calendar token from the Keychain")?;
    clear_cached_access();
    Ok(())
}

/// Module-level access-token cache (ADR-0010: access tokens live in memory
/// only; only the refresh token is persisted, and only to the Keychain).
struct CachedAccess {
    token: String,
    expires_at: DateTime<Utc>,
}

static ACCESS_CACHE: Mutex<Option<CachedAccess>> = Mutex::new(None);

fn cached_access_token() -> Option<String> {
    let guard = ACCESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .filter(|c| c.expires_at - chrono::Duration::seconds(EXPIRY_SLACK_SECS) > Utc::now())
        .map(|c| c.token.clone())
}

fn cache_access(bundle: &TokenBundle) {
    let mut guard = ACCESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(CachedAccess {
        token: bundle.access_token.clone(),
        expires_at: bundle.expires_at,
    });
}

fn clear_cached_access() {
    let mut guard = ACCESS_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *guard = None;
}

/// Refresh the access token through the stored refresh token and update the
/// in-memory cache. Split from [`get_access_token`] so tests can point the
/// client's token URL at a local mock endpoint.
async fn refresh_with_client(client: &BasicClient, store: &dyn SecretStore) -> Result<TokenBundle> {
    let refresh = store
        .get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT)
        .context("could not read the Google Calendar token from the Keychain")?
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            anyhow!("Google Calendar is not connected. Connect your Google account in Settings.")
        })?;

    let token = request_refresh(client, &refresh).await?;

    // Google occasionally rotates the refresh token on refresh; persist the
    // replacement best-effort (the old one may already be dead, but failing
    // the whole refresh over a Keychain hiccup would be worse than retrying
    // rotation on the next refresh).
    if let Some(new_refresh) = token.refresh_token() {
        if new_refresh.secret() != &refresh {
            if let Err(e) = persist_refresh_token(store, new_refresh.secret()) {
                log::warn!("google oauth: could not persist a rotated refresh token: {e}");
            }
        }
    }

    let bundle = TokenBundle {
        access_token: token.access_token().secret().clone(),
        expires_at: expires_at_from_now(token.expires_in()),
    };
    cache_access(&bundle);
    Ok(bundle)
}

/// One refresh-grant round trip, mapping Google's `invalid_grant` to the
/// distinct [`AuthError::InvalidGrant`] marker.
async fn request_refresh(
    client: &BasicClient,
    refresh_token: &str,
) -> Result<oauth2::basic::BasicTokenResponse> {
    client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
        .request_async(async_http_client)
        .await
        .map_err(|e| match &e {
            RequestTokenError::ServerResponse(resp)
                if *resp.error() == BasicErrorResponseType::InvalidGrant =>
            {
                anyhow::Error::new(AuthError::InvalidGrant)
            }
            // Display of the other variants carries endpoint/JSON error info,
            // never token material.
            _ => anyhow!("Google token refresh failed: {e}"),
        })
}

/// Persist the refresh token — Keychain or nothing (ADR-0010: no DB/sentinel
/// fallback; a failed write aborts the connect and nothing is stored).
fn persist_refresh_token(store: &dyn SecretStore, token: &str) -> Result<()> {
    store
        .set(secrets::GCAL_REFRESH_TOKEN_ACCOUNT, token)
        .map_err(|e| {
            anyhow!(
                "could not save the Google Calendar connection to the Keychain: {e}. \
                 Nothing was stored — please try connecting again."
            )
        })
}

fn expires_at_from_now(expires_in: Option<Duration>) -> DateTime<Utc> {
    let secs = expires_in
        .unwrap_or(Duration::from_secs(DEFAULT_EXPIRY_SECS))
        .as_secs()
        .min(i64::MAX as u64) as i64;
    Utc::now() + chrono::Duration::seconds(secs)
}

// ---------------------------------------------------------------------------
// OAuth client / auth-URL construction
// ---------------------------------------------------------------------------

/// Build the oauth2 client. `token_url` is a parameter (not the constant) so
/// tests can aim the exchange/refresh at a local mock endpoint.
fn build_client(
    client_id: &str,
    client_secret: Option<&str>,
    token_url: &str,
    redirect_uri: &str,
) -> Result<BasicClient> {
    Ok(BasicClient::new(
        ClientId::new(client_id.to_string()),
        client_secret.map(|s| ClientSecret::new(s.to_string())),
        AuthUrl::new(AUTH_ENDPOINT.to_string()).context("invalid Google auth endpoint")?,
        Some(TokenUrl::new(token_url.to_string()).context("invalid Google token endpoint")?),
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri.to_string()).context("invalid loopback redirect URI")?,
    ))
}

/// Build the consent URL: fresh PKCE S256 challenge + random `state`, the
/// ADR-0010 scope pair, and `access_type=offline` + `prompt=consent` so Google
/// actually issues a refresh token.
fn build_auth_request(client: &BasicClient) -> (String, CsrfToken, PkceCodeVerifier) {
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge)
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent");
    for scope in SCOPES {
        request = request.add_scope(Scope::new(scope.to_string()));
    }
    let (url, state) = request.url();
    (url.to_string(), state, pkce_verifier)
}

// ---------------------------------------------------------------------------
// Loopback redirect listener
// ---------------------------------------------------------------------------

const SUCCESS_PAGE: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Nixon</title></head>\
<body style=\"font-family:-apple-system,sans-serif;display:flex;align-items:center;justify-content:center;height:90vh\">\
<p>You&rsquo;re connected &mdash; you can close this window and return to Nixon.</p></body></html>";

const FAILURE_PAGE: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Nixon</title></head>\
<body style=\"font-family:-apple-system,sans-serif;display:flex;align-items:center;justify-content:center;height:90vh\">\
<p>The connection didn&rsquo;t complete. You can close this window and try again from Nixon&rsquo;s Settings.</p></body></html>";

/// Wait (bounded by `timeout`) for the OAuth redirect on the loopback
/// listener and return the authorization code. Timeout ⇒ the clean
/// [`AuthError::Cancelled`] marker (user abandoned the consent screen).
async fn wait_for_redirect(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    match tokio::time::timeout(timeout, redirect_accept_loop(listener, expected_state)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(anyhow::Error::new(AuthError::Cancelled)),
    }
}

/// Accept connections until one carries the OAuth redirect (browsers also poke
/// the port for `/favicon.ico` etc. — those get a 404 and we keep waiting),
/// answer it with a minimal HTML page, and return the code.
async fn redirect_accept_loop(listener: TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut stream, _peer) = listener
            .accept()
            .await
            .context("the local Google sign-in listener failed while waiting for the browser")?;

        let target = match read_request_target(&mut stream).await {
            Ok(t) => t,
            Err(_) => continue, // malformed/portscan noise — keep waiting
        };
        let params = parse_query_params(&target);
        if !params.contains_key("code") && !params.contains_key("error") {
            let _ = write_http_response(&mut stream, "404 Not Found", "").await;
            continue;
        }

        // From here this is the final answer, whatever it is.
        if let Some(error) = params.get("error") {
            let _ = write_http_response(&mut stream, "200 OK", FAILURE_PAGE).await;
            if error == "access_denied" {
                return Err(anyhow::Error::new(AuthError::Cancelled));
            }
            return Err(anyhow!(
                "Google returned an authorization error ({error}). Please try connecting again."
            ));
        }

        if params.get("state").map(String::as_str) != Some(expected_state) {
            let _ = write_http_response(&mut stream, "400 Bad Request", FAILURE_PAGE).await;
            return Err(anyhow!(
                "the Google sign-in redirect failed its state check (stale or tampered \
                 request). Please try connecting again."
            ));
        }

        match params.get("code").filter(|c| !c.is_empty()) {
            Some(code) => {
                let _ = write_http_response(&mut stream, "200 OK", SUCCESS_PAGE).await;
                return Ok(code.clone());
            }
            None => {
                let _ = write_http_response(&mut stream, "400 Bad Request", FAILURE_PAGE).await;
                return Err(anyhow!(
                    "Google's redirect did not include an authorization code. \
                     Please try connecting again."
                ));
            }
        }
    }
}

/// Read just the HTTP request line and return its target (e.g.
/// `/?code=…&state=…`). Bounded at 8 KiB — the redirect is tiny.
async fn read_request_target(stream: &mut TcpStream) -> Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 512];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .context("read from loopback redirect failed")?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(2).any(|w| w == b"\r\n") || buf.len() > 8192 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let request_line = text.lines().next().unwrap_or("");
    let target = request_line
        .split_whitespace()
        .nth(1)
        .context("malformed HTTP request on the loopback redirect")?;
    Ok(target.to_string())
}

/// Percent-decoded query parameters of a request target like `/?a=1&b=2`.
fn parse_query_params(target: &str) -> HashMap<String, String> {
    url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map(|u| {
            u.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

// ---------------------------------------------------------------------------
// Tests — MemoryStore + local mock endpoints only; no Keychain, no network.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;
    use std::sync::atomic::Ordering;

    /// Serializes tests that touch the module-level ACCESS_CACHE so parallel
    /// test threads can't observe each other's cached tokens. Async-aware
    /// (tokio Mutex) because the guard is held across awaits in the tests.
    static CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_cache() -> tokio::sync::MutexGuard<'static, ()> {
        let guard = CACHE_LOCK.lock().await;
        clear_cached_access();
        guard
    }

    /// One-shot local HTTP responder; returns the URL to aim a client at.
    async fn spawn_mock_endpoint(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                // Drain what arrives in the first read (headers + small form
                // body land together for these tiny requests), then respond.
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        format!("http://{addr}/token")
    }

    fn test_client(token_url: &str) -> BasicClient {
        build_client("test-client-id", None, token_url, "http://127.0.0.1:1").unwrap()
    }

    // --- PKCE / state / scopes -------------------------------------------

    #[test]
    fn auth_request_carries_pkce_s256_state_scopes_and_offline_access() {
        let client = test_client("http://127.0.0.1:1/token");
        let (auth_url, state, verifier) = build_auth_request(&client);
        let url = url::Url::parse(&auth_url).unwrap();
        let params: HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        assert_eq!(params.get("code_challenge_method").unwrap(), "S256");
        // RFC 7636: a base64url (no padding) SHA-256 digest is 43 chars.
        assert_eq!(params.get("code_challenge").unwrap().len(), 43);
        // The challenge must be derived from (never equal to) the verifier.
        assert_ne!(params.get("code_challenge").unwrap(), verifier.secret());
        assert_eq!(params.get("state").unwrap(), state.secret());
        assert_eq!(params.get("access_type").unwrap(), "offline");
        assert_eq!(params.get("prompt").unwrap(), "consent");
        let scope = params.get("scope").unwrap();
        for s in SCOPES {
            assert!(scope.contains(s), "missing scope {s} in {scope}");
        }
        // The ADR-0010 amendment's best-effort enrichment scopes must be present
        // alongside the calendar pair (requested at consent; used only when the
        // org actually grants runtime access — see [`SCOPES`]).
        assert_eq!(
            SCOPES.len(),
            4,
            "calendar pair + the two best-effort scopes"
        );
        assert!(scope.contains("cloud-identity.groups.readonly"));
        assert!(scope.contains("directory.readonly"));
        assert!(scope.contains("calendar.events.readonly"));
        assert!(scope.contains("calendar.calendarlist.readonly"));
        assert_eq!(params.get("response_type").unwrap(), "code");
    }

    #[test]
    fn auth_request_state_and_pkce_are_fresh_per_flow() {
        let client = test_client("http://127.0.0.1:1/token");
        let (_, state_a, verifier_a) = build_auth_request(&client);
        let (_, state_b, verifier_b) = build_auth_request(&client);
        assert_ne!(state_a.secret(), state_b.secret());
        assert_ne!(verifier_a.secret(), verifier_b.secret());
    }

    // --- Loopback redirect listener --------------------------------------

    async fn send_redirect(addr: std::net::SocketAddr, target: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn redirect_listener_returns_code_and_serves_close_page() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wait = tokio::spawn(async move {
            wait_for_redirect(listener, "state-1", Duration::from_secs(5)).await
        });
        let response = send_redirect(addr, "/?state=state-1&code=code-abc").await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("you can close this window"));
        assert_eq!(wait.await.unwrap().unwrap(), "code-abc");
    }

    #[tokio::test]
    async fn redirect_listener_rejects_state_mismatch() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wait = tokio::spawn(async move {
            wait_for_redirect(listener, "expected-state", Duration::from_secs(5)).await
        });
        let response = send_redirect(addr, "/?state=wrong&code=code-abc").await;
        assert!(response.starts_with("HTTP/1.1 400"));
        let err = wait.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("state check"), "got: {err}");
    }

    #[tokio::test]
    async fn redirect_listener_maps_access_denied_to_cancelled() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wait =
            tokio::spawn(
                async move { wait_for_redirect(listener, "s", Duration::from_secs(5)).await },
            );
        let _ = send_redirect(addr, "/?error=access_denied").await;
        let err = wait.await.unwrap().unwrap_err();
        assert!(is_cancelled(&err));
    }

    #[tokio::test]
    async fn redirect_listener_ignores_stray_requests_and_keeps_waiting() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let wait = tokio::spawn(async move {
            wait_for_redirect(listener, "state-2", Duration::from_secs(5)).await
        });
        let favicon = send_redirect(addr, "/favicon.ico").await;
        assert!(favicon.starts_with("HTTP/1.1 404"));
        let _ = send_redirect(addr, "/?state=state-2&code=real-code").await;
        assert_eq!(wait.await.unwrap().unwrap(), "real-code");
    }

    #[tokio::test]
    async fn abandoned_connect_times_out_as_cancelled() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let err = wait_for_redirect(listener, "s", Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(is_cancelled(&err));
    }

    // --- Refresh flow (mocked token endpoint) -----------------------------

    #[tokio::test]
    async fn refresh_flow_returns_token_from_mocked_endpoint() {
        let url = spawn_mock_endpoint(
            "200 OK",
            r#"{"access_token":"at-fresh","token_type":"Bearer","expires_in":3600}"#,
        )
        .await;
        let bundle = request_refresh(&test_client(&url), "rt-stored")
            .await
            .unwrap();
        assert_eq!(bundle.access_token().secret(), "at-fresh");
    }

    #[tokio::test]
    async fn refresh_invalid_grant_maps_to_distinct_marker() {
        let url = spawn_mock_endpoint("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
        let err = request_refresh(&test_client(&url), "rt-revoked")
            .await
            .unwrap_err();
        assert!(is_invalid_grant(&err));
        assert!(!is_cancelled(&err));
    }

    #[tokio::test]
    async fn refresh_reads_stored_token_and_caches_access_token() {
        let _guard = lock_cache().await;
        let store = MemoryStore::new();
        store
            .set(secrets::GCAL_REFRESH_TOKEN_ACCOUNT, "rt-stored")
            .unwrap();
        let url = spawn_mock_endpoint(
            "200 OK",
            r#"{"access_token":"at-cached","token_type":"Bearer","expires_in":3600}"#,
        )
        .await;
        let bundle = refresh_with_client(&test_client(&url), &store)
            .await
            .unwrap();
        assert_eq!(bundle.access_token, "at-cached");
        // The refreshed token is now served from the in-memory cache.
        assert_eq!(cached_access_token().as_deref(), Some("at-cached"));
    }

    #[tokio::test]
    async fn refresh_persists_a_rotated_refresh_token() {
        let _guard = lock_cache().await;
        let store = MemoryStore::new();
        store
            .set(secrets::GCAL_REFRESH_TOKEN_ACCOUNT, "rt-old")
            .unwrap();
        let url = spawn_mock_endpoint(
            "200 OK",
            r#"{"access_token":"at-x","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-rotated"}"#,
        )
        .await;
        refresh_with_client(&test_client(&url), &store)
            .await
            .unwrap();
        assert_eq!(
            store
                .get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT)
                .unwrap()
                .as_deref(),
            Some("rt-rotated")
        );
    }

    #[tokio::test]
    async fn refresh_without_stored_token_says_not_connected() {
        let _guard = lock_cache().await;
        let store = MemoryStore::new();
        let err = refresh_with_client(&test_client("http://127.0.0.1:1/token"), &store)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not connected"), "got: {err}");
    }

    #[tokio::test]
    async fn get_access_token_serves_from_cache_without_touching_store() {
        let _guard = lock_cache().await;
        cache_access(&TokenBundle {
            access_token: "at-live".into(),
            expires_at: Utc::now() + chrono::Duration::seconds(600),
        });
        // A failing store proves the cache path never reads the Keychain.
        let store = MemoryStore::new();
        store.fail.store(true, Ordering::Relaxed);
        let token = get_access_token(&store).await.unwrap();
        assert_eq!(token, "at-live");
    }

    #[tokio::test]
    async fn expired_cache_entry_is_not_served() {
        let _guard = lock_cache().await;
        cache_access(&TokenBundle {
            access_token: "at-stale".into(),
            expires_at: Utc::now() + chrono::Duration::seconds(EXPIRY_SLACK_SECS - 5),
        });
        assert_eq!(cached_access_token(), None);
    }

    // --- Persistence (ADR-0010: Keychain or nothing) ----------------------

    #[test]
    fn refresh_token_round_trips_through_the_secret_store() {
        let store = MemoryStore::new();
        persist_refresh_token(&store, "rt-round-trip").unwrap();
        assert_eq!(
            store
                .get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT)
                .unwrap()
                .as_deref(),
            Some("rt-round-trip")
        );
    }

    #[test]
    fn failed_keychain_write_aborts_and_persists_nothing() {
        let store = MemoryStore::new();
        store.fail.store(true, Ordering::Relaxed);
        let err = persist_refresh_token(&store, "rt-secret").unwrap_err();
        // Actionable, and never echoes the token value.
        let msg = err.to_string();
        assert!(msg.contains("Keychain"), "got: {msg}");
        assert!(
            !msg.contains("rt-secret"),
            "error text must not leak the token"
        );
        store.fail.store(false, Ordering::Relaxed);
        assert_eq!(
            store.get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT).unwrap(),
            None
        );
    }

    // --- Full connect finish() against mocks ------------------------------

    #[tokio::test]
    async fn finish_exchanges_code_and_persists_refresh_token() {
        let _guard = lock_cache().await;
        let token_url = spawn_mock_endpoint(
            "200 OK",
            r#"{"access_token":"at-first","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-first"}"#,
        )
        .await;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (_, _, pkce_verifier) = build_auth_request(&test_client(&token_url));
        let pending = PendingConnect {
            auth_url: String::new(),
            listener,
            client: test_client(&token_url),
            expected_state: CsrfToken::new("connect-state".into()),
            pkce_verifier,
            timeout: Duration::from_secs(5),
        };

        let store = MemoryStore::new();
        let redirect = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            send_redirect(addr, "/?state=connect-state&code=auth-code").await
        };
        let (result, page) = tokio::join!(pending.finish(&store), redirect);
        let bundle = result.unwrap();
        assert!(page.contains("you can close this window"));
        assert_eq!(bundle.access_token, "at-first");
        assert_eq!(
            store
                .get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT)
                .unwrap()
                .as_deref(),
            Some("rt-first")
        );
        // The fresh access token is cached for the immediate first sync.
        assert_eq!(cached_access_token().as_deref(), Some("at-first"));
    }

    #[tokio::test]
    async fn finish_with_failing_keychain_aborts_and_persists_nothing() {
        let _guard = lock_cache().await;
        let token_url = spawn_mock_endpoint(
            "200 OK",
            r#"{"access_token":"at-x","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-x"}"#,
        )
        .await;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (_, _, pkce_verifier) = build_auth_request(&test_client(&token_url));
        let pending = PendingConnect {
            auth_url: String::new(),
            listener,
            client: test_client(&token_url),
            expected_state: CsrfToken::new("s".into()),
            pkce_verifier,
            timeout: Duration::from_secs(5),
        };
        let store = MemoryStore::new();
        store.fail.store(true, Ordering::Relaxed);
        let redirect = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            send_redirect(addr, "/?state=s&code=auth-code").await
        };
        let (result, _page) = tokio::join!(pending.finish(&store), redirect);
        assert!(result.is_err());
        store.fail.store(false, Ordering::Relaxed);
        assert_eq!(
            store.get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT).unwrap(),
            None
        );
    }

    // --- Disconnect --------------------------------------------------------

    #[tokio::test]
    async fn revoke_and_clear_deletes_token_and_cache_even_when_revoke_endpoint_is_down() {
        let _guard = lock_cache().await;
        let store = MemoryStore::new();
        store
            .set(secrets::GCAL_REFRESH_TOKEN_ACCOUNT, "rt-gone")
            .unwrap();
        cache_access(&TokenBundle {
            access_token: "at-gone".into(),
            expires_at: Utc::now() + chrono::Duration::seconds(600),
        });
        // Unroutable revoke endpoint: best-effort revoke fails, cleanup wins.
        revoke_and_clear_at(&store, "http://127.0.0.1:1/revoke")
            .await
            .unwrap();
        assert_eq!(
            store.get(secrets::GCAL_REFRESH_TOKEN_ACCOUNT).unwrap(),
            None
        );
        assert_eq!(cached_access_token(), None);
    }

    #[test]
    fn token_bundle_debug_redacts_the_token() {
        let bundle = TokenBundle {
            access_token: "super-secret-access-token".into(),
            expires_at: Utc::now(),
        };
        let debug = format!("{bundle:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("super-secret-access-token"));
    }
}
