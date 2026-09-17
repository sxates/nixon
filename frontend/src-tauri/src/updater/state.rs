//! specs/0058 — pure update state machine. No Tauri, no I/O, fully unit-tested.

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};

/// What the frontend sees (`update-status` event + `api_get_update_status`).
///
/// The wire shape is a contract with `frontend/src/contexts/UpdateStatusContext.tsx`;
/// `status_json_is_the_typescript_contract` below pins every variant byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum UpdateStatus {
    Idle {
        last_checked: Option<DateTime<Utc>>,
    },
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    /// Downloaded, signature-verified, written to disk — waiting for the user to restart.
    /// `last_checked` rides along so About keeps its "checked …" line while staged.
    Ready {
        version: String,
        notes: String,
        last_checked: Option<DateTime<Utc>>,
    },
    Error {
        message: String,
        last_checked: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallRefusal {
    NothingStaged,
    RecordingInProgress,
}

impl std::fmt::Display for InstallRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingStaged => write!(f, "No update is ready to install"),
            Self::RecordingInProgress => write!(f, "Finish the recording first"),
        }
    }
}

/// The staged payload's record. `digest` is the SHA-256 of the exact bytes the plugin
/// signature-verified in memory, so install time can prove the file on disk is still
/// those bytes without going back to the network (see `super::verify`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Staged {
    version: String,
    notes: String,
    digest: [u8; 32],
    last_checked: Option<DateTime<Utc>>,
}

/// The pure machine. A staged (Ready) payload is remembered separately from the
/// visible status so a failed attempt at a *newer* version falls back to it.
#[derive(Debug, Default)]
pub struct UpdaterCore {
    status: Option<UpdateStatus>, // None == Idle{None}; keeps Default derivable
    staged: Option<Staged>,
}

impl UpdaterCore {
    pub fn status(&self) -> &UpdateStatus {
        self.status.as_ref().unwrap_or(&IDLE_NEVER)
    }

    fn set(&mut self, s: UpdateStatus) {
        self.status = Some(s);
    }

    /// The Ready status describing whatever is staged right now.
    fn staged_status(staged: &Staged) -> UpdateStatus {
        UpdateStatus::Ready {
            version: staged.version.clone(),
            notes: staged.notes.clone(),
            last_checked: staged.last_checked,
        }
    }

    fn last_checked(&self) -> Option<DateTime<Utc>> {
        match self.status() {
            UpdateStatus::Idle { last_checked } => *last_checked,
            UpdateStatus::Ready { last_checked, .. } => *last_checked,
            UpdateStatus::Error { last_checked, .. } => Some(*last_checked),
            _ => None,
        }
    }

    /// A check is starting. Silent while a payload is staged (the row keeps saying Ready).
    pub fn begin_check(&mut self) {
        if self.staged.is_none() {
            self.set(UpdateStatus::Checking);
        }
    }

    /// The manifest offers `version`. Download unless it is not newer than what is
    /// already staged. Unparseable versions are never downloaded.
    pub fn should_download(&self, version: &str) -> bool {
        let Ok(offered) = Version::parse(version) else {
            return false;
        };
        match &self.staged {
            Some(staged) => Version::parse(&staged.version)
                .map(|s| offered > s)
                .unwrap_or(true),
            None => true,
        }
    }

    pub fn begin_download(&mut self, version: &str) {
        self.set(UpdateStatus::Downloading {
            version: version.to_string(),
            received: 0,
            total: None,
        });
    }

    pub fn progress(&mut self, chunk: u64, content_length: Option<u64>) {
        if let Some(UpdateStatus::Downloading {
            received, total, ..
        }) = self.status.as_mut()
        {
            *received += chunk;
            if content_length.is_some() {
                *total = content_length;
            }
        }
    }

    /// A verified payload is on disk. `digest` is the SHA-256 of the verified bytes.
    pub fn ready(&mut self, version: &str, notes: &str, digest: [u8; 32], now: DateTime<Utc>) {
        let staged = Staged {
            version: version.to_string(),
            notes: notes.to_string(),
            digest,
            last_checked: Some(now),
        };
        self.set(Self::staged_status(&staged));
        self.staged = Some(staged);
    }

    pub fn found_none(&mut self, now: DateTime<Utc>) {
        match self.staged.as_mut() {
            // Still offering the staged payload, but the check itself did happen.
            Some(staged) => {
                staged.last_checked = Some(now);
                let status = Self::staged_status(staged);
                self.set(status);
            }
            None => self.set(UpdateStatus::Idle {
                last_checked: Some(now),
            }),
        }
    }

    pub fn failed(&mut self, message: &str, now: DateTime<Utc>) {
        match self.staged.as_mut() {
            Some(staged) => {
                staged.last_checked = Some(now);
                let status = Self::staged_status(staged);
                self.set(status);
            }
            None => self.set(UpdateStatus::Error {
                message: message.to_string(),
                last_checked: now,
            }),
        }
    }

    /// Install gate: something staged AND the recorder is stopped. Returns the version.
    pub fn can_install(&self, recording_stopped: bool) -> Result<String, InstallRefusal> {
        let staged = self.staged.as_ref().ok_or(InstallRefusal::NothingStaged)?;
        if !recording_stopped {
            return Err(InstallRefusal::RecordingInProgress);
        }
        Ok(staged.version.clone())
    }

    /// Forget the staged payload: its file is gone from disk, or the manifest has moved
    /// on. The visible status drops back to Idle so the UI stops offering a restart, and
    /// because `should_download` no longer sees a staged version the next check
    /// re-downloads the same version instead of skipping it forever.
    pub fn clear_staged(&mut self, now: DateTime<Utc>) {
        self.staged = None;
        self.set(UpdateStatus::Idle {
            last_checked: Some(now),
        });
    }

    /// Version of the staged payload, if any (tray menu reads this).
    pub fn staged_version(&self) -> Option<&str> {
        self.staged.as_ref().map(|s| s.version.as_str())
    }

    /// SHA-256 of the staged payload's verified bytes, if any (install checks the file
    /// on disk against this instead of trusting it).
    pub fn staged_digest(&self) -> Option<[u8; 32]> {
        self.staged.as_ref().map(|s| s.digest)
    }

    pub fn last_checked_at(&self) -> Option<DateTime<Utc>> {
        self.last_checked()
    }
}

static IDLE_NEVER: UpdateStatus = UpdateStatus::Idle { last_checked: None };

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn t0() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0).unwrap()
    }

    fn t1() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 16, 18, 0, 0).unwrap()
    }

    const D: [u8; 32] = [7u8; 32];

    fn ready_status(version: &str, notes: &str, at: chrono::DateTime<Utc>) -> UpdateStatus {
        UpdateStatus::Ready {
            version: version.into(),
            notes: notes.into(),
            last_checked: Some(at),
        }
    }

    #[test]
    fn starts_idle_never_checked() {
        let core = UpdaterCore::default();
        assert_eq!(core.status(), &UpdateStatus::Idle { last_checked: None });
    }

    #[test]
    fn idle_check_found_nothing_records_time() {
        let mut core = UpdaterCore::default();
        core.begin_check();
        assert_eq!(core.status(), &UpdateStatus::Checking);
        core.found_none(t0());
        assert_eq!(
            core.status(),
            &UpdateStatus::Idle {
                last_checked: Some(t0())
            }
        );
    }

    #[test]
    fn download_progress_then_ready() {
        let mut core = UpdaterCore::default();
        core.begin_check();
        assert!(core.should_download("0.3.0"));
        core.begin_download("0.3.0");
        core.progress(10, Some(100));
        assert_eq!(
            core.status(),
            &UpdateStatus::Downloading {
                version: "0.3.0".into(),
                received: 10,
                total: Some(100)
            }
        );
        core.progress(90, Some(100));
        assert_eq!(
            core.status(),
            &UpdateStatus::Downloading {
                version: "0.3.0".into(),
                received: 100,
                total: Some(100)
            }
        );
        core.ready("0.3.0", "notes", D, t0());
        assert_eq!(core.status(), &ready_status("0.3.0", "notes", t0()));
    }

    #[test]
    fn ready_remembers_the_verified_digest_until_it_is_cleared() {
        let mut core = UpdaterCore::default();
        assert_eq!(core.staged_digest(), None);
        core.ready("0.3.0", "", D, t0());
        assert_eq!(core.staged_digest(), Some(D));
        core.clear_staged(t0());
        assert_eq!(core.staged_digest(), None);
    }

    #[test]
    fn ready_is_not_redownloaded_and_check_is_silent() {
        let mut core = UpdaterCore::default();
        core.begin_download("0.3.0");
        core.ready("0.3.0", "", D, t0());
        core.begin_check(); // scheduled tick while Ready: stays Ready, never shows Checking
        assert!(matches!(core.status(), UpdateStatus::Ready { .. }));
        assert!(!core.should_download("0.3.0"));
        assert!(!core.should_download("0.2.9"));
        core.found_none(t1());
        // Still Ready, but About's "checked …" line advances to the newer check.
        assert_eq!(core.status(), &ready_status("0.3.0", "", t1()));
        assert_eq!(core.last_checked_at(), Some(t1()));
    }

    #[test]
    fn newer_manifest_supersedes_staged_and_failure_restores_it() {
        let mut core = UpdaterCore::default();
        core.begin_download("0.3.0");
        core.ready("0.3.0", "old", D, t0());
        assert!(core.should_download("0.3.1"));
        core.begin_download("0.3.1");
        assert!(matches!(core.status(), UpdateStatus::Downloading { .. }));
        core.failed("boom", t1());
        // The 0.3.0 payload is still staged and valid — keep offering it.
        assert_eq!(core.status(), &ready_status("0.3.0", "old", t1()));
        assert_eq!(core.staged_digest(), Some(D));
    }

    #[test]
    fn failure_without_staged_payload_is_error_and_next_check_clears_it() {
        let mut core = UpdaterCore::default();
        core.begin_check();
        core.failed("offline", t0());
        assert_eq!(
            core.status(),
            &UpdateStatus::Error {
                message: "offline".into(),
                last_checked: t0()
            }
        );
        core.begin_check();
        assert_eq!(core.status(), &UpdateStatus::Checking);
    }

    #[test]
    fn unparseable_versions_never_download() {
        let core = UpdaterCore::default();
        assert!(!core.should_download("not-a-version"));
    }

    #[test]
    fn install_gate() {
        let mut core = UpdaterCore::default();
        assert_eq!(core.can_install(true), Err(InstallRefusal::NothingStaged));
        core.begin_download("0.3.0");
        core.ready("0.3.0", "", D, t0());
        assert_eq!(
            core.can_install(false),
            Err(InstallRefusal::RecordingInProgress)
        );
        assert_eq!(core.can_install(true), Ok("0.3.0".to_string()));
    }

    #[test]
    fn clearing_a_staged_payload_makes_it_downloadable_again() {
        let mut core = UpdaterCore::default();
        core.begin_download("0.3.0");
        core.ready("0.3.0", "notes", D, t0());
        assert!(!core.should_download("0.3.0"));

        core.clear_staged(t0());

        // The same version must now be offered again — the file it referred to is gone.
        assert!(core.should_download("0.3.0"));
        assert_eq!(core.staged_version(), None);
        assert_eq!(core.can_install(true), Err(InstallRefusal::NothingStaged));
        assert_eq!(
            core.status(),
            &UpdateStatus::Idle {
                last_checked: Some(t0())
            }
        );
    }

    /// The exact JSON the webview parses. Changing any of these strings is a breaking
    /// change to `UpdateStatus` in `frontend/src/contexts/UpdateStatusContext.tsx`.
    #[test]
    fn status_json_is_the_typescript_contract() {
        let cases: [(UpdateStatus, &str); 5] = [
            (
                UpdateStatus::Idle { last_checked: None },
                r#"{"state":"idle","last_checked":null}"#,
            ),
            (UpdateStatus::Checking, r#"{"state":"checking"}"#),
            (
                UpdateStatus::Downloading {
                    version: "0.3.0".into(),
                    received: 1,
                    total: None,
                },
                r#"{"state":"downloading","version":"0.3.0","received":1,"total":null}"#,
            ),
            (
                ready_status("0.3.0", "notes", t0()),
                r#"{"state":"ready","version":"0.3.0","notes":"notes","last_checked":"2026-09-16T12:00:00Z"}"#,
            ),
            (
                UpdateStatus::Error {
                    message: "offline".into(),
                    last_checked: t0(),
                },
                r#"{"state":"error","message":"offline","last_checked":"2026-09-16T12:00:00Z"}"#,
            ),
        ];
        for (status, want) in cases {
            assert_eq!(serde_json::to_string(&status).unwrap(), want);
        }
    }
}
