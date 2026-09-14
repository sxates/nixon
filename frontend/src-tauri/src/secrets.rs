//! Keychain-backed secret store for LLM/transcription API keys (spec 0030 WS1,
//! ADR-0009).
//!
//! Storage model:
//! - The macOS Keychain is the **primary** store. The Keychain *service* is the
//!   bundle identifier (`ai.vinyl.app` / `ai.vinyl.app.debug`, via
//!   [`crate::app_paths::bundle_identifier`]) so dev and prod items are isolated
//!   (ADR-0004). *Accounts* are namespaced per table: `llm.<provider>` for
//!   summarization keys, `transcript.<provider>` for transcription keys — the two
//!   SQLite tables share column names (`groqApiKey`, `openaiApiKey`), so the
//!   namespace prevents collisions.
//! - SQLite keeps the column, but a migrated key is replaced with the sentinel
//!   [`KEYCHAIN_SENTINEL`]. A non-sentinel DB value is a legacy/fallback cleartext
//!   key and still works.
//!
//! Read semantics ([`resolve_secret`]): prefer the Keychain; fall back to a
//! non-sentinel DB value; a sentinel with an unreadable Keychain item means the
//! provider is **not configured** (return `None`, never an error). This graceful
//! degradation matters because ad-hoc-signed dev rebuilds change the code-signing
//! identity and can lose access to previously written Keychain items.
//!
//! Everything is behind the [`SecretStore`] trait so unit tests use
//! [`MemoryStore`] instead of the real (prompty, CI-unavailable) Keychain.

use anyhow::Result;

/// DB column value marking "the real key lives in the Keychain".
pub const KEYCHAIN_SENTINEL: &str = "__keychain_v1__";

/// Mask character used for UI hints (also used to detect placeholder echoes).
const MASK_CHAR: char = '\u{2022}'; // •

/// True when a DB value is the Keychain sentinel (not a real key).
pub fn is_sentinel(value: &str) -> bool {
    value == KEYCHAIN_SENTINEL
}

/// True for values that are placeholders rather than real keys: the DB sentinel
/// or a masked UI hint (`••••1234`). Save paths must ignore these so a frontend
/// echoing its masked display value never overwrites the stored key.
pub fn is_placeholder(value: &str) -> bool {
    is_sentinel(value) || value.starts_with(MASK_CHAR)
}

/// Build the safe UI hint for a configured key: `••••` + last 4 chars for keys
/// long enough that the tail reveals nothing, bare `••••` otherwise.
pub fn mask_key(key: &str) -> String {
    let mask = MASK_CHAR.to_string().repeat(4);
    if key.chars().count() > 8 {
        let tail: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{mask}{tail}")
    } else {
        mask
    }
}

/// Keychain account for the Google Calendar OAuth **refresh token** (specs/0032,
/// ADR-0010). Unlike the API-key accounts above there is **no sentinel/DB
/// fallback** for this account: the token lives in the Keychain or nowhere — a
/// failed Keychain write must abort the connect, and the token must never be
/// written to SQLite or logs under any failure mode.
pub const GCAL_REFRESH_TOKEN_ACCOUNT: &str = "gcal.refresh_token";

/// Keychain account name for a summarization-provider key.
pub fn llm_account(provider: &str) -> String {
    format!("llm.{provider}")
}

/// Keychain account name for a transcription-provider key.
pub fn transcript_account(provider: &str) -> String {
    format!("transcript.{provider}")
}

/// Minimal secret-store abstraction so the resolution/migration logic is unit
/// testable without the real Keychain.
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` = no item; `Err` = the store itself failed (locked, denied…).
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    /// Deleting a missing item is `Ok(())`.
    fn delete(&self, account: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Global store (initialized once at startup from the Tauri setup hook)
// ---------------------------------------------------------------------------

static STORE: std::sync::OnceLock<Box<dyn SecretStore>> = std::sync::OnceLock::new();

/// Initialize the global Keychain-backed store from the bundle identifier.
/// No-op when the identifier is unavailable or off macOS — callers of [`store`]
/// then degrade to DB-only behavior (identical to pre-0030 releases).
pub fn init_from_bundle_identifier() {
    #[cfg(target_os = "macos")]
    {
        let Some(identifier) = crate::app_paths::bundle_identifier() else {
            log::warn!(
                "secrets: bundle identifier not initialized; Keychain disabled, API keys stay in SQLite"
            );
            return;
        };
        let store = KeychainStore::new(identifier.to_string());
        if STORE.set(Box::new(store)).is_err() {
            log::warn!("secrets: store already initialized; keeping the first value");
        } else {
            log::info!("secrets: Keychain store initialized (service '{identifier}')");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        log::info!("secrets: no Keychain backend on this platform; API keys stay in SQLite");
    }
}

/// The process-wide secret store, or `None` when uninitialized (unit tests,
/// non-macOS): callers must fall back to the DB value.
pub fn store() -> Option<&'static dyn SecretStore> {
    STORE.get().map(|b| b.as_ref())
}

// ---------------------------------------------------------------------------
// macOS Keychain implementation (via the `keyring` crate, apple-native)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub struct KeychainStore {
    service: String,
}

#[cfg(target_os = "macos")]
impl KeychainStore {
    pub fn new(service: String) -> Self {
        Self { service }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(|e| {
            anyhow::anyhow!(
                "could not open Keychain entry '{}' / '{}': {e}",
                self.service,
                account
            )
        })
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(
                "Keychain read failed for '{account}': {e}. If this is a dev (ad-hoc signed) \
                 build, the item may belong to a previous binary signature — re-enter the key \
                 in Settings to fix it."
            )),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.entry(account)?
            .set_password(secret)
            .map_err(|e| anyhow::anyhow!("Keychain write failed for '{account}': {e}"))
    }

    fn delete(&self, account: &str) -> Result<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!(
                "Keychain delete failed for '{account}': {e}"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory fake for unit tests (Keychain is unavailable/prompty in CI)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MemoryStore {
    items: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// When true every operation fails — simulates a locked/denied Keychain.
    pub fail: std::sync::atomic::AtomicBool,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn check(&self) -> Result<()> {
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            anyhow::bail!("memory secret store: simulated failure");
        }
        Ok(())
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        self.check()?;
        Ok(self.items.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.check()?;
        self.items
            .lock()
            .unwrap()
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.check()?;
        self.items.lock().unwrap().remove(account);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Resolution / persistence semantics (shared by all read & write sites)
// ---------------------------------------------------------------------------

/// Resolve the effective API key for one account.
///
/// Order: Keychain first, then a non-sentinel DB value. A sentinel DB value with
/// no readable Keychain item means **not configured** — `None`, never an error
/// (dev ad-hoc re-signs can orphan Keychain items; summarization must degrade to
/// "provider not configured", not crash).
pub fn resolve_secret(
    store: Option<&dyn SecretStore>,
    account: &str,
    db_value: Option<String>,
) -> Option<String> {
    if let Some(store) = store {
        match store.get(account) {
            Ok(Some(secret)) if !secret.is_empty() => return Some(secret),
            Ok(_) => {}
            Err(e) => log::warn!("secrets: {e}; falling back to database value for '{account}'"),
        }
    }
    match db_value {
        Some(v) if !v.is_empty() && !is_sentinel(&v) => Some(v),
        _ => None,
    }
}

/// Persist a key and return the value the DB column should hold: the sentinel
/// when the Keychain write succeeded, the cleartext key otherwise (fallback —
/// never lose a key the user just entered).
pub fn db_value_for_save(store: Option<&dyn SecretStore>, account: &str, key: &str) -> String {
    if let Some(store) = store {
        match store.set(account, key) {
            Ok(()) => return KEYCHAIN_SENTINEL.to_string(),
            Err(e) => log::warn!(
                "secrets: {e}; storing the key in SQLite as a fallback (it will be re-migrated \
                 to the Keychain on a later launch)"
            ),
        }
    }
    key.to_string()
}

/// Best-effort Keychain deletion (used when a provider key is removed).
pub fn delete_secret(store: Option<&dyn SecretStore>, account: &str) {
    if let Some(store) = store {
        if let Err(e) = store.delete(account) {
            log::warn!("secrets: {e} (the DB value was still cleared)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_key_shows_only_last_four_for_long_keys() {
        assert_eq!(mask_key("sk-abcdefgh1234"), "••••1234");
        assert_eq!(mask_key("short"), "••••");
        assert_eq!(mask_key("12345678"), "••••"); // exactly 8: tail would reveal half
    }

    #[test]
    fn placeholders_are_detected() {
        assert!(is_placeholder(KEYCHAIN_SENTINEL));
        assert!(is_placeholder("••••1234"));
        assert!(is_placeholder("••••"));
        assert!(!is_placeholder("sk-real-key"));
        assert!(!is_placeholder(""));
    }

    #[test]
    fn resolve_prefers_keychain_over_db() {
        let store = MemoryStore::new();
        store.set("llm.openai", "keychain-key").unwrap();
        let got = resolve_secret(Some(&store), "llm.openai", Some("db-cleartext".to_string()));
        assert_eq!(got.as_deref(), Some("keychain-key"));
    }

    #[test]
    fn resolve_falls_back_to_cleartext_db_value() {
        let store = MemoryStore::new();
        let got = resolve_secret(Some(&store), "llm.groq", Some("db-cleartext".to_string()));
        assert_eq!(got.as_deref(), Some("db-cleartext"));
        // Also without any store at all (tests / non-macOS).
        let got = resolve_secret(None, "llm.groq", Some("db-cleartext".to_string()));
        assert_eq!(got.as_deref(), Some("db-cleartext"));
    }

    #[test]
    fn sentinel_with_unreadable_keychain_means_not_configured() {
        let store = MemoryStore::new();
        store.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        let got = resolve_secret(
            Some(&store),
            "llm.claude",
            Some(KEYCHAIN_SENTINEL.to_string()),
        );
        assert_eq!(got, None);
        // Same when the store has simply lost the item.
        let ok_store = MemoryStore::new();
        let got = resolve_secret(
            Some(&ok_store),
            "llm.claude",
            Some(KEYCHAIN_SENTINEL.to_string()),
        );
        assert_eq!(got, None);
    }

    #[test]
    fn sentinel_still_resolves_when_keychain_read_fails_but_db_holds_cleartext() {
        let store = MemoryStore::new();
        store.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        let got = resolve_secret(Some(&store), "llm.openai", Some("cleartext".to_string()));
        assert_eq!(got.as_deref(), Some("cleartext"));
    }

    #[test]
    fn db_value_for_save_uses_sentinel_on_success_and_cleartext_on_failure() {
        let store = MemoryStore::new();
        let v = db_value_for_save(Some(&store), "llm.openai", "sk-key");
        assert_eq!(v, KEYCHAIN_SENTINEL);
        assert_eq!(store.get("llm.openai").unwrap().as_deref(), Some("sk-key"));

        let failing = MemoryStore::new();
        failing
            .fail
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let v = db_value_for_save(Some(&failing), "llm.openai", "sk-key");
        assert_eq!(v, "sk-key");

        let v = db_value_for_save(None, "llm.openai", "sk-key");
        assert_eq!(v, "sk-key");
    }

    #[test]
    fn accounts_are_namespaced_per_table() {
        assert_ne!(llm_account("groq"), transcript_account("groq"));
        assert_eq!(llm_account("openai"), "llm.openai");
        assert_eq!(transcript_account("deepgram"), "transcript.deepgram");
    }
}
