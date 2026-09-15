//! Action-item candidate extraction: the LLM call + owner resolution (specs/0034).
//!
//! The LLM returns CANDIDATES only — `{"description", "assignee", "due"}` strings. Rust
//! owns identity, owner resolution, and the diff: assignee strings are resolved against
//! the participant roster + the owner's identity in code ([`resolve_candidates`]); the
//! model never sees or emits IDs. The prompt embeds the roster (display names + emails)
//! plus the `"me"` option carrying the owner's known identity — name and emails — because
//! the roster excludes self by design (migrations/20260629000000) and the summary text
//! renders the owner as "You" (diarization default) or their real name, never as "me".
//!
//! The user prompt additionally carries an ALREADY TRACKED section (the protected
//! existing rows' descriptions) so the model suppresses re-extractions of tasks the user
//! already owns — the semantic dedup layer above the diff's token matcher (2026-07
//! template-switch incident: rephrasings like "Set up the room" → "Look at the room" sit
//! below any sane token-similarity threshold).

use std::future::Future;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::warn;
use unicode_normalization::UnicodeNormalization;

use crate::action_items::diff::ResolvedCandidate;
use crate::action_items::reply_parse::{self, parse_candidates};
use crate::database::repositories::meeting_participant::MeetingParticipant;
use crate::database::repositories::owner_emails::OwnerEmailsRepository;
use crate::database::repositories::people::PeopleRepository;
use crate::people::enroll::OWNER_PERSON_ID;
use crate::summary::provider_config::ProviderConfig;

/// One candidate as the LLM emits it: a bare JSON object of three strings. `assignee` is
/// expected to be a roster display name / email, the literal `"me"`, or null; `due` is a
/// verbatim hint ("Friday", "2026-07-10") — no date parsing in v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItemCandidate {
    pub description: String,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
}

/// The meeting owner's known identity, used at BOTH ends of owner resolution (specs/0034
/// §Owner resolution): the prompt presents the `"me"` option with these strings, and
/// [`resolve_assignee`] maps any of them back to `assignee_is_self`. Needed because the
/// summary text never says "me" — it renders the owner as "You" (diarization
/// local-speaker default) or as their real display name after a rename.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnerIdentity {
    /// The owner person's display name (`people` row [`OWNER_PERSON_ID`], "You" by
    /// default), when that row exists.
    pub display_name: Option<String>,
    /// The owner's declared email addresses (`owner_emails`, normalized).
    pub emails: Vec<String>,
}

impl OwnerIdentity {
    /// Load the owner's identity from the DB: the singleton owner `people` row (may not
    /// exist yet — created lazily by voiceprint enrollment) + all declared owner emails.
    pub async fn load(pool: &SqlitePool) -> anyhow::Result<Self> {
        let display_name = PeopleRepository::get(pool, OWNER_PERSON_ID)
            .await
            .context("Failed to load the owner identity for assignee resolution")?
            .map(|p| p.display_name);
        let emails = OwnerEmailsRepository::list(pool)
            .await
            .context("Failed to load owner emails for assignee resolution")?;
        Ok(Self {
            display_name,
            emails,
        })
    }
}

/// Case/normalization fold for identity comparison: NFC (consistent with
/// `diff::normalize_description` — combining vs precomposed accents must not miss),
/// trimmed, lowercased.
fn fold(text: &str) -> String {
    text.nfc().collect::<String>().trim().to_lowercase()
}

/// `"Alice Example (alice@example.com)"` → `"Alice Example"`: strip ONE trailing
/// parenthetical, for models that copy a whole ASSIGNEES line verbatim. `None` when there
/// is no trailing parenthetical or nothing would remain.
fn strip_trailing_parenthetical(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    if !trimmed.ends_with(')') {
        return None;
    }
    let open = trimmed.rfind('(')?;
    let head = trimmed[..open].trim_end();
    (!head.is_empty()).then_some(head)
}

/// Resolves one extracted assignee string against the meeting roster and the owner's
/// identity (specs/0034 Owner resolution — pure Rust, never trusting the model with IDs).
/// All comparisons are NFC-folded + case-insensitive:
/// - `"me"`, `"you"`, the owner's display name, or any owner email →
///   `assignee_is_self` (the app owner is NOT a `people` row);
/// - a roster display name, email, or the composed `Name (email)` form exactly as the
///   prompt renders it → that `person_id`;
/// - a string with one trailing parenthetical is re-tried without it (verbatim copies of
///   a whole ASSIGNEES line, e.g. `"me" — ...` never reaches here but
///   `Alice Example (alice@example.com)` does); wrapping double quotes are ignored;
/// - anything else → kept verbatim in `assignee_raw` (display fallback, user can fix it);
/// - `None` / blank → unassigned.
pub fn resolve_assignee(
    assignee: Option<&str>,
    roster: &[MeetingParticipant],
    owner: &OwnerIdentity,
) -> (Option<String>, bool, Option<String>) {
    let Some(raw) = assignee.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, false, None);
    };

    if let Some(resolved) = try_resolve(raw, roster, owner) {
        return resolved;
    }
    // Fallback: strip one trailing parenthetical and re-match ("Alice (alice@x.com)",
    // "\"me\" (the meeting owner)" — shapes a copy-verbatim model produces).
    if let Some(head) = strip_trailing_parenthetical(raw) {
        if let Some(resolved) = try_resolve(head, roster, owner) {
            return resolved;
        }
    }

    (None, false, Some(raw.to_string()))
}

/// One resolution attempt over an exact candidate string (quotes stripped, then folded).
/// `None` = no match at this shape (the caller may retry a reduced shape).
#[allow(clippy::type_complexity)] // the (person_id, is_self, raw) triple is the module's write-shape
fn try_resolve(
    text: &str,
    roster: &[MeetingParticipant],
    owner: &OwnerIdentity,
) -> Option<(Option<String>, bool, Option<String>)> {
    // The prompt renders the owner option as `"me"` (quoted); tolerate a verbatim copy.
    let folded = fold(text.trim().trim_matches('"'));
    if folded.is_empty() {
        return None;
    }

    // Owner forms first (spec: "me"/owner-name → is_self; the roster excludes self by
    // design, so an owner-name collision with a participant is not expected).
    let is_owner = folded == "me"
        || folded == "you"
        || owner
            .display_name
            .as_deref()
            .is_some_and(|name| fold(name) == folded)
        || owner.emails.iter().any(|email| fold(email) == folded);
    if is_owner {
        return Some((None, true, None));
    }

    for participant in roster {
        let name = fold(&participant.display_name);
        let email = participant
            .email
            .as_deref()
            .map(fold)
            .filter(|e| !e.is_empty());
        let name_match = !name.is_empty() && name == folded;
        let email_match = email.as_deref() == Some(folded.as_str());
        // The composed form exactly as the prompt's ASSIGNEES list renders it.
        let composed_match = email
            .as_deref()
            .is_some_and(|e| format!("{name} ({e})") == folded);
        if name_match || email_match || composed_match {
            return Some((Some(participant.person_id.clone()), false, None));
        }
    }

    None
}

/// Parses a due hint into a structured ISO date (`YYYY-MM-DD`) IFF the whole trimmed hint
/// is exactly that — a strict, unambiguous match (specs/0038 WS1.a). Returns `None` for
/// fuzzy hints ("Friday", "next week") and for date-times; those stay hint-only.
fn parse_iso_due_date(hint: &str) -> Option<String> {
    let hint = hint.trim();
    chrono::NaiveDate::parse_from_str(hint, "%Y-%m-%d")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// Turns raw LLM candidates into the diff/persistence shape: trims and drops empty
/// descriptions, resolves each assignee via [`resolve_assignee`], and carries the due
/// hint through verbatim (plus a best-effort structured [`parse_iso_due_date`]).
pub fn resolve_candidates(
    candidates: Vec<ActionItemCandidate>,
    roster: &[MeetingParticipant],
    owner: &OwnerIdentity,
) -> Vec<ResolvedCandidate> {
    candidates
        .into_iter()
        .filter_map(|c| {
            let description = c.description.trim().to_string();
            if description.is_empty() {
                return None;
            }
            let (assignee_person_id, assignee_is_self, assignee_raw) =
                resolve_assignee(c.assignee.as_deref(), roster, owner);
            let due_hint = c
                .due
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty());
            // Best-effort structured date (specs/0038 WS1.a): only when the hint is an
            // unambiguous full ISO date (YYYY-MM-DD). Anything fuzzy ("Friday", "next
            // week") stays hint-only; the user sets the structured value from the UI.
            let due_date = due_hint.as_deref().and_then(parse_iso_due_date);
            Some(ResolvedCandidate {
                description,
                assignee_person_id,
                assignee_is_self,
                assignee_raw,
                due_hint,
                due_date,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Prompt construction (specs/0034 LLM contract — prompts are code; document intent)
// ---------------------------------------------------------------------------

/// The fixed head of the extraction system prompt. Design intent:
/// - **Object-wrapped contract (`{"action_items": [...]}`), stated twice** (here and again
///   at the end of the user prompt): small local models drift into prose unless the output
///   shape is the loudest instruction in the prompt. specs/0054 W1: this was a BARE ARRAY
///   until v1.18, which contradicted the `response_format: {"type":"json_object"}` that
///   `deterministic_json` sets for every OpenAI-compatible provider (`llm_client.rs`).
///   Ollama enforces that as a decoding constraint, so `gemma4:26b` answered with a single
///   bare `{"description":…,"assignee":…,"due":…}` — unparseable, AND collapsing three
///   real action items into one. Both attempts failed identically because extraction runs
///   at `temperature: 0.0`, so the retry is byte-for-byte the same reply.
/// - **Exactly three keys, shown literally**: mirrors [`ActionItemCandidate`] so a strict
///   `serde_json` parse is the only contract check we need.
/// - **Commitments only**: the summary also contains decisions/discussion; extracting
///   those would flood the task hub with non-tasks.
/// - **Verbatim `due`, never invented**: v1 stores the hint as-is (`due_hint`, no date
///   parsing), so a hallucinated ISO date would look authoritative while being wrong.
/// - The roster + the `"me"` owner option are appended by [`build_system_prompt`]; the
///   model copies strings, it never sees IDs ([`resolve_assignee`] owns resolution).
const SYSTEM_PROMPT_HEADER: &str = "You are the action-item extractor for a meeting assistant. You will be given a meeting's summary and, when available, notes written by the meeting owner. Identify every concrete action item: a task that someone committed to do, was assigned, or that the meeting agreed must happen.\n\nRespond with ONLY a JSON object with exactly one key, \"action_items\", whose value is an array — no prose, no explanations, no markdown, no code fences. Each element of that array must be an object with exactly these three keys:\n{\"action_items\": [{\"description\": string, \"assignee\": string or null, \"due\": string or null}]}\n\nRules:\n- description: one short, self-contained imperative sentence (e.g. \"Send the revised deck to the client\"). Merge repeated mentions of the same task into a single item.\n- assignee: copy one name or email verbatim from the ASSIGNEES list below. Use \"me\" for tasks the meeting owner took on (first-person commitments such as \"I'll send it\", or tasks the text assigns to the owner by name or as \"You\"). If the responsible person is clearly named in the text but missing from the list, copy their name exactly as written. Use null when nobody specific is responsible.\n- due: the due date or timing hint exactly as stated in the text (e.g. \"Friday\", \"by end of Q3\", \"2026-07-10\"). Use null when none is stated. Never invent dates.\n- Do not include decisions, opinions, discussion points, open questions, or work that is already done.\n- If there are no action items, return {\"action_items\": []}.\n\nASSIGNEES:";

/// Appended to the retry's user prompt after the first reply failed to parse (specs/0034:
/// one retry, then give up). A fresh single-shot call — `generate_summary` keeps no
/// conversation state — so the reminder rides on the same prompt.
const RETRY_REMINDER: &str = "\n\nIMPORTANT: Your previous reply could not be parsed. Return ONLY the JSON object — starting with { and ending with } — whose single key \"action_items\" holds the array, with no other text before or after it.";

/// Cap on the ALREADY TRACKED list embedded in the user prompt: bounds prompt size for
/// long-lived meetings that accumulate many protected items (~50 one-line descriptions ≈
/// a few hundred tokens — negligible next to the summary itself, even on small local
/// models). Overflow items simply lose prompt-level dedup and fall back to the diff's
/// token matcher + protected rules, which remain the hard safety guarantee.
pub const ALREADY_TRACKED_CAP: usize = 50;

/// Header + instruction for the ALREADY TRACKED section of the user prompt. Design intent
/// (the second dedup layer, above [`crate::action_items::diff`]'s token matcher): a
/// template switch can rephrase a tracked task beyond any token-level similarity
/// ("Set up the room" → "Look at the room" — the 2026-07 dogfood incident's below-
/// threshold pair), so the model itself is told which commitments are already tracked and
/// asked to suppress them, HOWEVER worded. "Same commitment ... even if worded
/// differently" targets semantic identity — the one judgment the model can make and the
/// Rust matcher cannot. The list is descriptions only (never IDs, statuses, or assignees:
/// nothing the model could echo back incorrectly).
///
/// The trailing counterweight exists because the suppress instruction alone
/// OVER-suppressed (2026-07 zero-candidate incident: the model returned `[]` for a
/// summary full of tasks, and the empty result read as "delete everything pristine").
/// The tie-break is stated explicitly — include when unsure — because the two failure
/// modes are wildly asymmetric: a false include is deduped downstream by the diff (and a
/// zero-candidate run is additionally quarantined by
/// `crate::action_items::skip_zero_candidate_deletes`), while a false omission deletes
/// the user's open items.
const ALREADY_TRACKED_HEADER: &str = "ALREADY TRACKED:\nThese tasks are already tracked from earlier extractions. Do NOT include any task that is the same commitment as one of these, even if worded differently. Only omit a task if it clearly matches one of these. When unsure, INCLUDE the task — duplicates are handled downstream. Never return an empty array just because tasks look similar to this list.";

/// The owner's ASSIGNEES entry: the copyable token is `"me"`; the dash-separated
/// description carries the owner's known identity (display name + emails) and the "You"
/// convention, because that's how the summary text actually refers to the owner.
/// [`resolve_assignee`] maps every one of those strings — and a verbatim copy with a
/// trailing parenthetical — back to `assignee_is_self`, so a model that copies the name
/// instead of `"me"` still resolves.
fn owner_assignee_line(owner: &OwnerIdentity) -> String {
    let mut identity: Vec<&str> = Vec::new();
    if let Some(name) = owner.display_name.as_deref().map(str::trim).filter(|n| {
        !n.is_empty() && !n.eq_ignore_ascii_case("you") && !n.eq_ignore_ascii_case("me")
    }) {
        identity.push(name);
    }
    identity.extend(
        owner
            .emails
            .iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty()),
    );
    if identity.is_empty() {
        "- \"me\" — the meeting owner; the text may call them \"You\"".to_string()
    } else {
        format!(
            "- \"me\" — the meeting owner ({}); the text may call them \"You\"",
            identity.join(", ")
        )
    }
}

/// System prompt = fixed contract + the owner's `"me"` entry + the meeting roster.
/// Roster entries are rendered as `- Name (email)` (email omitted when absent) so the
/// model can copy either part or the whole line — [`resolve_assignee`] matches the name,
/// the email, and the composed `Name (email)` form. The `"me"` entry is always present
/// (the roster excludes the app owner by design); an empty roster still yields a valid
/// prompt where `"me"`, verbatim unlisted names, and null are the only options.
fn build_system_prompt(roster: &[MeetingParticipant], owner: &OwnerIdentity) -> String {
    let mut prompt = String::from(SYSTEM_PROMPT_HEADER);
    prompt.push('\n');
    prompt.push_str(&owner_assignee_line(owner));
    for participant in roster {
        let name = participant.display_name.trim();
        match participant
            .email
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            Some(email) => {
                prompt.push_str(&format!("\n- {name} ({email})"));
            }
            None => {
                prompt.push_str(&format!("\n- {name}"));
            }
        }
    }
    prompt
}

/// User prompt = the extraction sources, labeled, then the dedup list. The summary is the
/// primary source (it already distilled commitments with speaker attribution); the
/// owner's notes are the second source (they catch commitments the summary dropped —
/// specs/0034 recall mitigation). `already_tracked` (protected rows' descriptions, capped
/// at [`ALREADY_TRACKED_CAP`]) renders as the ALREADY TRACKED section — omitted entirely
/// when empty, so first extractions pay nothing. The closing line restates the output
/// contract because trailing instructions are the ones small models obey best.
fn build_user_prompt(
    summary_markdown: &str,
    user_notes: Option<&str>,
    already_tracked: &[&str],
) -> String {
    let mut prompt = format!("MEETING SUMMARY:\n\n{summary_markdown}");
    if let Some(notes) = user_notes.map(str::trim).filter(|n| !n.is_empty()) {
        prompt.push_str(&format!("\n\nMEETING OWNER'S NOTES:\n\n{notes}"));
    }
    let tracked: Vec<&str> = already_tracked
        .iter()
        .map(|d| d.trim())
        .filter(|d| !d.is_empty())
        .take(ALREADY_TRACKED_CAP)
        .collect();
    if !tracked.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(ALREADY_TRACKED_HEADER);
        for description in tracked {
            prompt.push_str(&format!("\n- {description}"));
        }
    }
    prompt.push_str("\n\nReturn ONLY the JSON array of action items.");
    prompt
}

// ---------------------------------------------------------------------------
// The call + single-retry loop, generic over the LLM seam for testability
// ---------------------------------------------------------------------------

/// The extraction conversation against an abstract LLM call `(system, user) → text`.
/// Private seam so tests drive canned model outputs; production wires it to
/// `generate_summary` in [`extract_candidates`].
///
/// Flow: one call → [`parse_candidates`]; on PARSE failure only, one retry with
/// [`RETRY_REMINDER`] appended; a second parse failure (or any transport error) is `Err`
/// — the caller logs and records nothing (background feature: never a user-facing error).
async fn extract_with_llm<F, Fut>(
    llm: F,
    summary_markdown: &str,
    user_notes: Option<&str>,
    already_tracked: &[&str],
    roster: &[MeetingParticipant],
    owner: &OwnerIdentity,
) -> anyhow::Result<Vec<ActionItemCandidate>>
where
    F: Fn(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let system_prompt = build_system_prompt(roster, owner);
    let user_prompt = build_user_prompt(summary_markdown, user_notes, already_tracked);

    let first_reply = llm(system_prompt.clone(), user_prompt.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Action-item extraction call failed: {e}"))?;
    let first_err = match parse_candidates(&first_reply) {
        Ok(candidates) => return Ok(candidates),
        Err(e) => e,
    };

    // Privacy: log the failure SHAPE, never the reply content (it derives from the
    // transcript). specs/0054 W1 added the leading character and the cap check —
    // between them they separate the three failure modes that all used to surface
    // as one opaque "extraction failed":
    //   '{' well under the cap  → wrong shape (the json_object/bare-array mismatch)
    //   '[' or '{' AT the cap   → truncation; raise DEFAULT_OLLAMA_MAX_TOKENS instead
    //   prose                   → the model ignored the contract entirely
    warn!(
        "Action-item extraction reply was unparseable ({}); retrying once with a JSON-only reminder: {:#}",
        reply_parse::describe_shape(&first_reply),
        first_err
    );

    let second_reply = llm(system_prompt, format!("{user_prompt}{RETRY_REMINDER}"))
        .await
        .map_err(|e| anyhow::anyhow!("Action-item extraction retry call failed: {e}"))?;
    parse_candidates(&second_reply)
        .context("Model returned unparseable action-item output twice; giving up on this run")
}

/// Extracts action-item candidates from a persisted summary (+ the user's notes) with one
/// small LLM call through `summary::llm_client::generate_summary` (specs/0034: ~1–3k
/// input tokens regardless of meeting length, un-chunked, template-agnostic,
/// privacy-neutral — the same provider already saw the full transcript).
///
/// `config` is the exact provider/model/key/endpoints the summary run used (decided
/// 2026-07-03: no separate extraction provider setting). `already_tracked` is the
/// PROTECTED existing rows' descriptions (see `run_extraction` for why pristine rows are
/// excluded) — the prompt-level dedup layer that catches rephrasings the diff's token
/// matcher can't ("Set up the room" → "Look at the room"). Model assumptions: the output
/// is a small JSON array, well within every provider's default output cap, so
/// `max_tokens` is left to the provider default (a low explicit cap could truncate
/// mid-array — truncated JSON fails the strict parse and burns the retry).
/// `temperature = 0.0` for deterministic extraction (the CustomOpenAI tuning knobs are
/// summary-only). No cancellation token: the call is short and the run is
/// fire-and-forget in the background.
#[allow(clippy::too_many_arguments)] // the extraction call's full input surface; callers pass it once
pub async fn extract_candidates(
    client: &reqwest::Client,
    config: &ProviderConfig,
    app_data_dir: Option<&PathBuf>,
    summary_markdown: &str,
    user_notes: Option<&str>,
    already_tracked: &[&str],
    roster: &[MeetingParticipant],
    owner: &OwnerIdentity,
) -> anyhow::Result<Vec<ActionItemCandidate>> {
    let llm = |system_prompt: String, user_prompt: String| async move {
        crate::summary::llm_client::generate_summary_with_options(
            client,
            &config.provider,
            &config.model_name,
            &config.api_key,
            &system_prompt,
            &user_prompt,
            config.ollama_endpoint.as_deref(),
            config.custom_openai_endpoint.as_deref(),
            None,      // max_tokens: provider default (an explicit cap risks truncated JSON)
            Some(0.0), // temperature: deterministic extraction (specs/0034)
            None,      // top_p: provider default
            app_data_dir,
            None, // no cancellation: short background call
            // specs/0053 task 4: the GBNF grammar is NOT wired (it crashes the BuiltInAI
            // sidecar — see grammar.rs's doc comment). `deterministic_json` still forces
            // greedy decoding + neutral penalties + response_format on hosted providers;
            // `parse_candidates` is the primary defence for well-formed output.
            &crate::summary::summary_engine::options::GenerationOptions::deterministic_json(),
        )
        .await
    };
    extract_with_llm(
        llm,
        summary_markdown,
        user_notes,
        already_tracked,
        roster,
        owner,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(person_id: &str, name: &str, email: Option<&str>) -> MeetingParticipant {
        MeetingParticipant {
            person_id: person_id.to_string(),
            display_name: name.to_string(),
            email: email.map(str::to_string),
            role: None,
            source: "calendar".to_string(),
            photo_data_uri: None,
        }
    }

    /// Owner with no known identity (no owner person row, no owner emails) — the
    /// pre-0018 / fresh-install shape.
    fn no_owner() -> OwnerIdentity {
        OwnerIdentity::default()
    }

    fn owner(name: Option<&str>, emails: &[&str]) -> OwnerIdentity {
        OwnerIdentity {
            display_name: name.map(str::to_string),
            emails: emails.iter().map(|e| e.to_string()).collect(),
        }
    }

    /// Verification row 9: roster name → person_id; unknown name → raw; "me" → self.
    #[test]
    fn resolves_roster_name_unknown_and_me() {
        let roster = vec![
            participant("p-alice", "Alice", Some("alice@example.com")),
            participant("p-bob", "Bob Jones", None),
        ];

        // Display-name match, case-insensitive.
        assert_eq!(
            resolve_assignee(Some("alice"), &roster, &no_owner()),
            (Some("p-alice".to_string()), false, None)
        );
        // Email match.
        assert_eq!(
            resolve_assignee(Some("ALICE@EXAMPLE.COM"), &roster, &no_owner()),
            (Some("p-alice".to_string()), false, None)
        );
        // Not on the roster → raw fallback.
        assert_eq!(
            resolve_assignee(Some("Carol"), &roster, &no_owner()),
            (None, false, Some("Carol".to_string()))
        );
        // The owner convention.
        assert_eq!(
            resolve_assignee(Some("Me"), &roster, &no_owner()),
            (None, true, None)
        );
        // Unassigned.
        assert_eq!(
            resolve_assignee(None, &roster, &no_owner()),
            (None, false, None)
        );
        assert_eq!(
            resolve_assignee(Some("   "), &roster, &no_owner()),
            (None, false, None)
        );
    }

    /// Spec §Owner resolution: the summary text calls the owner "You" (diarization
    /// default) or their real name after a rename — every owner-identity form must land
    /// on `assignee_is_self`, never on a raw string or a fabricated person.
    #[test]
    fn resolves_owner_name_you_and_email_to_self() {
        let roster = vec![participant("p-alice", "Alice", Some("alice@example.com"))];
        let owner = owner(Some("Ada"), &["ada@x.com", "b@personal.io"]);

        for form in [
            "You",
            "you",
            "YOU",
            "Ada",
            "ada",
            "ada@x.com",
            "B@PERSONAL.IO",
        ] {
            assert_eq!(
                resolve_assignee(Some(form), &roster, &owner),
                (None, true, None),
                "owner form {form:?} must resolve to self"
            );
        }
        // The default owner display name ("You") — same outcome via the literal check.
        let default_owner = self::owner(Some("You"), &[]);
        assert_eq!(
            resolve_assignee(Some("You"), &roster, &default_owner),
            (None, true, None)
        );
        // Roster matching still wins for non-owner participants.
        assert_eq!(
            resolve_assignee(Some("Alice"), &roster, &owner),
            (Some("p-alice".to_string()), false, None)
        );
    }

    /// Finding: a model that copies a whole ASSIGNEES line verbatim must still resolve —
    /// the composed `Name (email)` form directly, and one trailing parenthetical is
    /// stripped as a fallback (`"me" (the meeting owner)`, `Ada (ada@x.com)`).
    #[test]
    fn resolves_verbatim_copies_of_assignees_lines() {
        let roster = vec![participant(
            "p-alice",
            "Alice Example",
            Some("alice@example.com"),
        )];
        let owner = owner(Some("Ada"), &["ada@x.com"]);

        // Composed roster line, any case.
        assert_eq!(
            resolve_assignee(Some("Alice Example (alice@example.com)"), &roster, &owner),
            (Some("p-alice".to_string()), false, None)
        );
        assert_eq!(
            resolve_assignee(Some("alice example (ALICE@EXAMPLE.COM)"), &roster, &owner),
            (Some("p-alice".to_string()), false, None)
        );
        // Quoted "me" and "me"-with-parenthetical (the old prompt's rendering).
        assert_eq!(
            resolve_assignee(Some("\"me\""), &roster, &owner),
            (None, true, None)
        );
        assert_eq!(
            resolve_assignee(Some("\"me\" (the meeting owner)"), &roster, &owner),
            (None, true, None)
        );
        // Owner name with a trailing parenthetical.
        assert_eq!(
            resolve_assignee(Some("Ada (ada@x.com)"), &roster, &owner),
            (None, true, None)
        );
        // An unknown name with a parenthetical stays raw VERBATIM (display fallback keeps
        // what the model wrote, not the stripped form).
        assert_eq!(
            resolve_assignee(Some("Marcus (Acme)"), &roster, &owner),
            (None, false, Some("Marcus (Acme)".to_string()))
        );
    }

    /// C2: name comparison is NFC-normalized, consistent with diff.rs — a combining-mark
    /// "Émile" from the model must match a precomposed roster "Émile".
    #[test]
    fn resolver_is_nfc_normalization_insensitive() {
        // Roster name precomposed (U+00C9), model output with combining accent (E + U+0301).
        let roster = vec![participant("p-emile", "\u{c9}mile", Some("emile@x.com"))];
        assert_eq!(
            resolve_assignee(Some("E\u{301}mile"), &roster, &no_owner()),
            (Some("p-emile".to_string()), false, None)
        );
        // Owner-name path too: combining-mark owner name vs precomposed model output.
        let owner = owner(Some("Ange\u{301}le"), &[]);
        assert_eq!(
            resolve_assignee(Some("Ang\u{e9}le"), &[], &owner),
            (None, true, None),
            "precomposed \u{e9} must match the owner's combining-mark name"
        );
    }

    #[test]
    fn resolve_candidates_trims_filters_and_resolves() {
        let roster = vec![participant("p-alice", "Alice", None)];
        let resolved = resolve_candidates(
            vec![
                ActionItemCandidate {
                    description: "  Send the deck  ".to_string(),
                    assignee: Some("Alice".to_string()),
                    due: Some(" Friday ".to_string()),
                },
                ActionItemCandidate {
                    description: "   ".to_string(), // empty → dropped
                    assignee: None,
                    due: None,
                },
                ActionItemCandidate {
                    description: "Book the room".to_string(),
                    assignee: Some("me".to_string()),
                    due: Some("".to_string()), // blank due → None
                },
                ActionItemCandidate {
                    description: "File the report".to_string(),
                    assignee: None,
                    due: Some("2026-07-10".to_string()), // ISO date → best-effort due_date
                },
            ],
            &roster,
            &no_owner(),
        );

        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].description, "Send the deck");
        assert_eq!(resolved[0].assignee_person_id.as_deref(), Some("p-alice"));
        assert!(!resolved[0].assignee_is_self);
        assert_eq!(resolved[0].due_hint.as_deref(), Some("Friday"));
        // A fuzzy hint stays hint-only — no structured date.
        assert!(resolved[0].due_date.is_none());
        assert_eq!(resolved[1].description, "Book the room");
        assert!(resolved[1].assignee_is_self);
        assert!(resolved[1].due_hint.is_none());
        // WS1.a — an unambiguous ISO hint is best-effort promoted to due_date; the hint stays.
        assert_eq!(resolved[2].due_hint.as_deref(), Some("2026-07-10"));
        assert_eq!(resolved[2].due_date.as_deref(), Some("2026-07-10"));
    }

    #[test]
    fn parse_iso_due_date_accepts_only_full_iso_dates() {
        assert_eq!(
            parse_iso_due_date("2026-07-10"),
            Some("2026-07-10".to_string())
        );
        assert_eq!(
            parse_iso_due_date("  2026-01-05  "),
            Some("2026-01-05".to_string())
        );
        // Fuzzy or partial hints are rejected (hint-only).
        assert!(parse_iso_due_date("Friday").is_none());
        assert!(parse_iso_due_date("next week").is_none());
        assert!(parse_iso_due_date("2026-07").is_none());
        assert!(parse_iso_due_date("2026-13-01").is_none()); // invalid month
        assert!(parse_iso_due_date("2026-07-10T09:00:00Z").is_none()); // date-time, not a date
    }

    #[test]
    fn candidate_json_shape_matches_llm_contract() {
        // The task-4 parser will deserialize exactly this shape.
        let parsed: Vec<ActionItemCandidate> = serde_json::from_str(
            r#"[{"description": "Send the deck", "assignee": "Alice", "due": null},
                {"description": "Book the room"}]"#,
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].assignee.as_deref(), Some("Alice"));
        assert!(parsed[1].assignee.is_none());
        assert!(parsed[1].due.is_none());
    }

    // =====================================================================
    // Prompt construction
    // =====================================================================

    #[test]
    fn system_prompt_embeds_roster_me_convention_and_json_contract() {
        let roster = vec![
            participant("p-alice", "Alice Example", Some("alice@example.com")),
            participant("p-bob", "Bob Jones", None),
        ];
        let prompt = build_system_prompt(&roster, &no_owner());

        // The output contract is stated literally. specs/0054 W1: it must be the
        // OBJECT-wrapped shape — a bare array contradicts the `json_object`
        // response_format that `deterministic_json` sets, and Ollama enforces that
        // as a hard decoding constraint.
        assert!(prompt.contains(r#"ONLY a JSON object with exactly one key, "action_items""#));
        assert!(prompt.contains(
            r#"{"action_items": [{"description": string, "assignee": string or null, "due": string or null}]}"#
        ));
        assert!(
            !prompt.contains("bare JSON array"),
            "the bare-array contract is what broke extraction on Ollama (specs/0054 W1)"
        );
        // The "me" convention and the roster, names + emails, IDs never.
        assert!(prompt.contains("- \"me\" — the meeting owner; the text may call them \"You\""));
        assert!(prompt.contains("- Alice Example (alice@example.com)"));
        assert!(prompt.contains("- Bob Jones"));
        assert!(
            !prompt.contains("p-alice"),
            "the model must never see person IDs"
        );
        // Due hints must be verbatim, never invented.
        assert!(prompt.contains("Never invent dates"));
    }

    /// Finding: the summary renders the owner as "You" or their real name, so the
    /// ASSIGNEES list must present the "me" option WITH the owner's identity when known.
    #[test]
    fn system_prompt_owner_line_carries_owner_identity() {
        let prompt = build_system_prompt(&[], &owner(Some("Ada"), &["ada@x.com"]));
        assert!(
            prompt.contains(
                "- \"me\" — the meeting owner (Ada, ada@x.com); the text may call them \"You\""
            ),
            "owner identity missing from the me-line:\n{prompt}"
        );
        // The default "You" display name is redundant with the trailing clause — skipped.
        let prompt = build_system_prompt(&[], &owner(Some("You"), &["me@work.com"]));
        assert!(prompt.contains(
            "- \"me\" — the meeting owner (me@work.com); the text may call them \"You\""
        ));
    }

    #[test]
    fn system_prompt_with_empty_roster_still_offers_me() {
        let prompt = build_system_prompt(&[], &no_owner());
        assert!(prompt.contains("- \"me\" — the meeting owner"));
        assert!(
            prompt.ends_with("the text may call them \"You\""),
            "no dangling roster lines"
        );
    }

    #[test]
    fn user_prompt_includes_summary_and_notes_when_present() {
        let with_notes = build_user_prompt("## Summary\n- do x", Some("remember the deck"), &[]);
        assert!(with_notes.contains("MEETING SUMMARY:\n\n## Summary\n- do x"));
        assert!(with_notes.contains("MEETING OWNER'S NOTES:\n\nremember the deck"));
        assert!(with_notes.ends_with("Return ONLY the JSON array of action items."));

        // Absent or blank notes → no notes section at all.
        let without = build_user_prompt("## Summary", None, &[]);
        assert!(!without.contains("MEETING OWNER'S NOTES"));
        let blank = build_user_prompt("## Summary", Some("   "), &[]);
        assert!(!blank.contains("MEETING OWNER'S NOTES"));
    }

    /// The prompt-level dedup layer (2026-07 template-switch incident): every protected
    /// row's description — the caller passes completed, dismissed, user-edited, AND
    /// manual alike — is listed under ALREADY TRACKED with the suppress instruction, and
    /// the JSON contract line stays the trailing instruction.
    #[test]
    fn user_prompt_lists_already_tracked_descriptions_with_instruction() {
        let tracked = [
            "Send the presentation deck to participants", // user-edited
            "Order pizza for the meeting",                // user-edited
            "Set up the room",                            // completed
            "Follow up with legal",                       // dismissed
            "Water the office plants",                    // manual
        ];
        let prompt = build_user_prompt("## Summary", Some("my notes"), &tracked);

        assert!(prompt.contains(
            "ALREADY TRACKED:\nThese tasks are already tracked from earlier extractions. \
             Do NOT include any task that is the same commitment as one of these, even if \
             worded differently."
        ));
        for description in tracked {
            assert!(
                prompt.contains(&format!("\n- {description}")),
                "tracked item {description:?} missing from the prompt"
            );
        }
        // Section order: sources, then the dedup list, then the trailing contract line.
        let notes_at = prompt.find("MEETING OWNER'S NOTES").unwrap();
        let tracked_at = prompt.find("ALREADY TRACKED:").unwrap();
        assert!(notes_at < tracked_at, "dedup list comes after the sources");
        assert!(prompt.ends_with("Return ONLY the JSON array of action items."));
    }

    /// Regression (2026-07 zero-candidate incident): the suppress instruction alone made
    /// the model return an empty array for a summary full of tasks. The ALREADY TRACKED
    /// section must carry the explicit counterweight — only omit on a clear match,
    /// include when unsure, never an empty array out of similarity — and the JSON
    /// contract line must remain the trailing instruction.
    #[test]
    fn already_tracked_section_carries_the_include_when_unsure_counterweight() {
        let prompt = build_user_prompt("## Summary", None, &["Set up the room"]);
        assert!(prompt.contains("Only omit a task if it clearly matches one of these."));
        assert!(
            prompt.contains("When unsure, INCLUDE the task — duplicates are handled downstream.")
        );
        assert!(prompt
            .contains("Never return an empty array just because tasks look similar to this list."));
        // The counterweight rides inside the ALREADY TRACKED section, before the list...
        let counterweight_at = prompt.find("When unsure, INCLUDE").unwrap();
        assert!(prompt.find("ALREADY TRACKED:").unwrap() < counterweight_at);
        assert!(counterweight_at < prompt.find("\n- Set up the room").unwrap());
        // ...and the JSON contract stays the trailing instruction (unchanged).
        assert!(prompt.ends_with("Return ONLY the JSON array of action items."));
    }

    #[test]
    fn user_prompt_omits_already_tracked_when_empty() {
        assert!(!build_user_prompt("## Summary", None, &[]).contains("ALREADY TRACKED"));
        // Blank-only entries count as empty too (nothing to suppress).
        assert!(!build_user_prompt("## Summary", None, &["  ", ""]).contains("ALREADY TRACKED"));
    }

    #[test]
    fn user_prompt_caps_already_tracked_list() {
        let many: Vec<String> = (0..ALREADY_TRACKED_CAP + 10)
            .map(|i| format!("tracked task number {i}"))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let prompt = build_user_prompt("## Summary", None, &refs);

        assert_eq!(
            prompt.matches("\n- tracked task number").count(),
            ALREADY_TRACKED_CAP,
            "list capped at ALREADY_TRACKED_CAP"
        );
        assert!(prompt.contains(&format!(
            "\n- tracked task number {}",
            ALREADY_TRACKED_CAP - 1
        )));
        assert!(
            !prompt.contains(&format!("\n- tracked task number {ALREADY_TRACKED_CAP}")),
            "entries past the cap are dropped"
        );
    }

    // =====================================================================
    // The call + single-retry loop (verification row 10, seam level)
    // =====================================================================

    /// A minimal parseable reply for the retry-loop tests below — the parse-shape
    /// fixtures (fenced, prose, `<think>`, object-wrapped) moved to `grammar.rs`
    /// alongside `parse_candidates` (specs/0053 — this file is ratchet-allowlisted
    /// and may only shrink). Only "did it parse" matters here, not the shape.
    const CLEAN_ARRAY: &str = r#"[{"description": "Send the deck to Alice", "assignee": "Alice", "due": "Friday"}, {"description": "Book the room", "assignee": "me", "due": null}]"#;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A canned-output LLM: returns `replies[n]` for call `n` and records every
    /// (system, user) prompt pair. `Err` entries simulate transport failures.
    struct FakeLlm {
        replies: Vec<Result<String, String>>,
        calls: AtomicUsize,
        prompts: Mutex<Vec<(String, String)>>,
    }

    impl FakeLlm {
        fn new(replies: Vec<Result<&str, &str>>) -> Self {
            Self {
                replies: replies
                    .into_iter()
                    .map(|r| r.map(str::to_string).map_err(str::to_string))
                    .collect(),
                calls: AtomicUsize::new(0),
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn call(&self, system: String, user: String) -> Result<String, String> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            self.prompts.lock().unwrap().push((system, user));
            self.replies
                .get(n)
                .cloned()
                .unwrap_or_else(|| Err("unexpected extra LLM call".to_string()))
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    async fn run_extraction_with(
        fake: &FakeLlm,
        summary: &str,
        notes: Option<&str>,
        roster: &[MeetingParticipant],
    ) -> anyhow::Result<Vec<ActionItemCandidate>> {
        extract_with_llm(
            |system, user| {
                let result = fake.call(system, user);
                async move { result }
            },
            summary,
            notes,
            &[],
            roster,
            &no_owner(),
        )
        .await
    }

    /// The ALREADY TRACKED list reaches the wire: it rides in the user prompt of the
    /// real call — and survives intact on the parse-failure retry (the retry re-sends
    /// the same sources + dedup list, only appending the JSON reminder).
    #[tokio::test]
    async fn already_tracked_rides_in_the_sent_prompt_and_its_retry() {
        let fake = FakeLlm::new(vec![Ok("not json"), Ok("[]")]);
        let tracked = ["Set up the room", "Order pizza for the meeting"];
        extract_with_llm(
            |system, user| {
                let result = fake.call(system, user);
                async move { result }
            },
            "## Summary",
            None,
            &tracked,
            &[],
            &no_owner(),
        )
        .await
        .unwrap();

        let prompts = fake.prompts.lock().unwrap();
        for (n, (_, user)) in prompts.iter().enumerate() {
            assert!(
                user.contains("ALREADY TRACKED:"),
                "call {n} carries the dedup section"
            );
            assert!(
                user.contains("\n- Set up the room"),
                "call {n} lists the tracked items"
            );
            assert!(user.contains("\n- Order pizza for the meeting"));
        }
    }

    #[tokio::test]
    async fn clean_first_reply_needs_no_retry() {
        let fake = FakeLlm::new(vec![Ok(CLEAN_ARRAY)]);
        let out = run_extraction_with(&fake, "## Summary", None, &[])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(fake.call_count(), 1, "no retry after a parseable reply");
    }

    #[tokio::test]
    async fn unparseable_first_reply_retries_once_with_reminder() {
        let fake = FakeLlm::new(vec![
            Ok("I could not find any structured tasks, sorry."),
            Ok(CLEAN_ARRAY),
        ]);
        let out = run_extraction_with(&fake, "## Summary", Some("my notes"), &[])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(fake.call_count(), 2);

        let prompts = fake.prompts.lock().unwrap();
        assert!(
            !prompts[0].1.contains("could not be parsed"),
            "first call carries no reminder"
        );
        assert!(
            prompts[1].1.ends_with(RETRY_REMINDER),
            "retry appends the JSON-only reminder"
        );
        assert_eq!(
            prompts[0].0, prompts[1].0,
            "system prompt unchanged on retry"
        );
        assert!(
            prompts[1].1.starts_with(&prompts[0].1),
            "retry keeps the original sources intact"
        );
    }

    /// Verification row 10: invalid twice → Err (the caller logs and records nothing).
    #[tokio::test]
    async fn unparseable_twice_returns_err_after_exactly_two_calls() {
        let fake = FakeLlm::new(vec![Ok("not json"), Ok("still not json")]);
        let err = run_extraction_with(&fake, "## Summary", None, &[])
            .await
            .expect_err("second parse failure must surface as Err");
        assert_eq!(fake.call_count(), 2, "exactly one retry, then give up");
        assert!(
            err.to_string().contains("twice"),
            "error names the double failure: {err:#}"
        );
    }

    #[tokio::test]
    async fn transport_error_is_err_without_retry() {
        // The retry exists for JSON discipline, not for provider outages: a transport
        // failure surfaces immediately ("Scan again" is the user-facing retry).
        let fake = FakeLlm::new(vec![Err("connection refused (is Ollama running?)")]);
        let err = run_extraction_with(&fake, "## Summary", None, &[])
            .await
            .expect_err("transport failure is an Err");
        assert_eq!(fake.call_count(), 1, "no retry on transport failure");
        assert!(err.to_string().contains("extraction call failed"));
    }

    #[tokio::test]
    async fn empty_array_reply_is_ok_and_empty() {
        let fake = FakeLlm::new(vec![Ok("[]")]);
        let out = run_extraction_with(&fake, "## Summary with no commitments", None, &[])
            .await
            .unwrap();
        assert!(out.is_empty());
        assert_eq!(fake.call_count(), 1);
    }

    #[tokio::test]
    async fn prompts_carry_summary_notes_and_roster() {
        let roster = vec![participant("p-alice", "Alice", Some("alice@example.com"))];
        let fake = FakeLlm::new(vec![Ok("[]")]);
        run_extraction_with(
            &fake,
            "## The Summary Body",
            Some("deck due friday"),
            &roster,
        )
        .await
        .unwrap();
        let prompts = fake.prompts.lock().unwrap();
        let (system, user) = &prompts[0];
        assert!(system.contains("- Alice (alice@example.com)"));
        assert!(user.contains("## The Summary Body"));
        assert!(user.contains("deck due friday"));
    }

    // =====================================================================
    // One representative end-to-end case: a realistic template-shaped summary + notes
    // through prompt → FakeLlm reply → parse → resolution. FakeLlm never reads the
    // prompt, so per-template fixtures added nothing (the parse SHAPES — fenced, prose,
    // <think>, object-wrapped — are pinned by the parse_* tests above); real-model
    // output quality is the manual smoke (specs/0034 Verification).
    // =====================================================================

    /// `templates/standard_meeting.json` shape — Summary / Key Decisions / Action Items
    /// (owner-task-due table). Covers every resolution outcome at once: roster name →
    /// person FK, roster email → person FK, "me" → self flag, off-roster name → raw
    /// fallback, verbatim due hints, notes as the second recall source.
    #[tokio::test]
    async fn eval_end_to_end_extraction_and_resolution() {
        let summary = "# Weekly Product Sync\n\n\
            ## Summary\n\
            The team reviewed the beta launch plan and agreed the pilot begins July 10.\n\n\
            ## Key Decisions\n\
            - Ship the beta to the pilot group on July 10.\n\n\
            ## Action Items\n\
            | **Owner** | Task | Due | Reference Transcript Segment | Segment Time stamp |\n\
            | --- | --- | --- | --- | --- |\n\
            | Alice | Send the revised pricing deck to pilot customers | Friday | \"I'll get the deck out\" | 00:12:41 |\n\
            | You | Book the launch retro room | — | \"I can grab a room\" | 00:31:05 |\n\
            | Priya Sharma | Publish the migration runbook | end of next week | \"I'll write it up\" | 00:35:12 |\n\
            | Dana | Draft the beta announcement email | July 8 | \"Dana volunteered\" | 00:38:20 |";
        let notes = "remember: deck goes out BEFORE the announcement";
        let reply = r#"[
            {"description": "Send the revised pricing deck to pilot customers", "assignee": "Alice Example", "due": "Friday"},
            {"description": "Book the launch retro room", "assignee": "me", "due": null},
            {"description": "Publish the migration runbook", "assignee": "priya@acme.com", "due": "end of next week"},
            {"description": "Draft the beta announcement email", "assignee": "Dana", "due": "July 8"}
        ]"#;

        let fake = FakeLlm::new(vec![Ok(reply)]);
        let roster = vec![
            participant("p-alice", "Alice Example", Some("alice@example.com")),
            participant("p-priya", "Priya Sharma", Some("priya@acme.com")),
        ];
        let candidates = run_extraction_with(&fake, summary, Some(notes), &roster)
            .await
            .unwrap();

        // The notes made it into the prompt (the second recall source).
        assert!(fake.prompts.lock().unwrap()[0]
            .1
            .contains("deck goes out BEFORE"));

        let resolved = resolve_candidates(candidates, &roster, &no_owner());
        assert_eq!(resolved.len(), 4);
        // Roster display name → person FK; verbatim due hint.
        assert_eq!(resolved[0].assignee_person_id.as_deref(), Some("p-alice"));
        assert_eq!(resolved[0].due_hint.as_deref(), Some("Friday"));
        // "me" → the app owner flag.
        assert!(resolved[1].assignee_is_self, "\"me\" → the app owner flag");
        assert!(resolved[1].due_hint.is_none());
        // Roster EMAIL → person FK; freeform due hint rides through (no date parsing).
        assert_eq!(resolved[2].assignee_person_id.as_deref(), Some("p-priya"));
        assert_eq!(resolved[2].due_hint.as_deref(), Some("end of next week"));
        // Off-roster name → raw fallback, never a fabricated person id.
        assert_eq!(resolved[3].assignee_person_id, None);
        assert_eq!(resolved[3].assignee_raw.as_deref(), Some("Dana"));
    }
}
