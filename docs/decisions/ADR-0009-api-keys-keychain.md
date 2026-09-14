# ADR-0009: API keys move to the macOS Keychain

Status: Accepted
Date: 2026-07-01

## Context

LLM and transcription API keys were stored as **cleartext** in SQLite: columns on
`settings` / `transcript_settings` (with *shared* column names across the two tables,
e.g. `groqApiKey`) plus a key nested inside the serialized `customOpenAIConfig` JSON
blob. The Tauri getters also returned raw keys to the webview. Spec 0028 deferred the
fix pending a Developer-ID-signed build (Keychain access is tied to the code-signing
identity); v1.3.0 ships signed + notarized (ADR-0008), unblocking it (spec 0030
WS1/WS2). The 0028 interim mitigation (0600 file perms) stays as defense-in-depth.

## Decision

**Store:** the `keyring` crate (v3, `apple-native` feature only) over raw
`security-framework` — it is a thin, maintained wrapper around the same
Security.framework keychain, gives us `get/set/delete` with typed errors (notably
`NoEntry`), and avoids hand-rolling CFString/SecItem plumbing for three operations.
Everything sits behind a `SecretStore` trait (`frontend/src-tauri/src/secrets.rs`)
with an in-memory fake for unit tests (the real Keychain is prompty/unavailable in CI).

**Service naming:** the Keychain *service* is the bundle identifier captured at
startup via `app_paths::init_bundle_identifier` (`ai.vinyl.app` prod,
`ai.vinyl.app.debug` dev), so dev/prod items are isolated in the ADR-0004 spirit.
*Accounts* are namespaced per table — `llm.<provider>` (summarization, incl.
`llm.custom-openai`) and `transcript.<provider>` — because the two SQLite tables share
column names.

**Sentinel + fallback semantics:**
- A migrated/saved key's DB value becomes the sentinel **`__keychain_v1__`**.
- *Reads* (all through `SettingsRepository`): Keychain first → non-sentinel DB value
  (legacy cleartext) → otherwise **not configured** (`Ok(None)`, never an error). A
  sentinel with an unreadable Keychain item therefore degrades to "provider not
  configured" instead of breaking summarization.
- *Writes*: Keychain first; on failure the cleartext key is kept in the DB (never lose
  a key the user just typed) and re-migrated later.
- *Startup migration* (`migrate_keys_to_keychain`, run after DB init and after legacy
  imports): idempotently moves every cleartext value (both tables + the JSON blob) to
  the Keychain; on any store failure the DB value is left untouched and a warning is
  logged.
- *Deletes* clear both the column and the Keychain item.

**IPC (WS2):** getters return `{ configured, masked }` (`••••` + last 4) —
`api_get_api_key`, `api_get_transcript_api_key`, and the key fields of
`api_get_model_config` / `api_get_transcript_config` / `api_get_custom_openai_config`.
Setting a key stays write-only; save paths **ignore masked/sentinel echoes** so a
frontend round-trip can never clobber a stored key, and saving a custom-openai config
without a key preserves the existing one (removal goes through
`api_delete_api_key("custom-openai")`). Backend calls that legitimately need the raw
key (model listing, connection test) resolve it server-side via
`api::resolve_provider_key` when the webview sends nothing/a mask.

## Consequences / caveats

- **Dev-build caveat:** the bare dev binary is ad-hoc signed and its identity changes
  per rebuild, so macOS may deny access to items written by a previous build. That
  surfaces as "provider not configured" (plus a log warning) — re-enter the key in
  Settings; no crash, no error spam.
- Clearing the custom-openai key alone is no longer possible from the settings form
  (blank now means "keep"); delete the config to remove it.
- Cleartext keys can persist in the DB only when the Keychain write itself failed.

## Manual verification needed on a signed production build

1. Upgrade path: install over a pre-1.3 DB with real keys → keys land in Keychain
   (Keychain Access shows service `ai.vinyl.app`), DB columns show `__keychain_v1__`,
   summaries still work for every configured provider.
2. Fresh install: save a key → summarize; no keychain permission prompt loops under
   the hardened runtime (no extra entitlement is expected for the app's own keychain
   items, but this is the assumption to validate).
3. App update (replace `Vinyl.app` via `upgrade-vinyl.sh`): same Developer ID →
   Keychain items remain readable without re-prompting.
4. Settings UI shows `••••last4` for configured providers; "replace key" flow works;
   Test Connection works for custom-openai without retyping the key.
