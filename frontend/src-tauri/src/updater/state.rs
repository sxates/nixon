//! specs/0058 — pure update state machine. No Tauri, no I/O, fully unit-tested.

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};

/// What the frontend sees (`update-status` event + `api_get_update_status`).
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
    Ready {
        version: String,
        notes: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Staged {
    version: String,
    notes: String,
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

    fn last_checked(&self) -> Option<DateTime<Utc>> {
        match self.status() {
            UpdateStatus::Idle { last_checked } => *last_checked,
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

    pub fn ready(&mut self, version: &str, notes: &str) {
        self.staged = Some(Staged {
            version: version.to_string(),
            notes: notes.to_string(),
        });
        self.set(UpdateStatus::Ready {
            version: version.to_string(),
            notes: notes.to_string(),
        });
    }

    pub fn found_none(&mut self, now: DateTime<Utc>) {
        if self.staged.is_none() {
            self.set(UpdateStatus::Idle {
                last_checked: Some(now),
            });
        }
    }

    pub fn failed(&mut self, message: &str, now: DateTime<Utc>) {
        match &self.staged {
            Some(s) => self.set(UpdateStatus::Ready {
                version: s.version.clone(),
                notes: s.notes.clone(),
            }),
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

    /// Version of the staged payload, if any (tray menu reads this).
    pub fn staged_version(&self) -> Option<&str> {
        self.staged.as_ref().map(|s| s.version.as_str())
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
        core.ready("0.3.0", "notes");
        assert_eq!(
            core.status(),
            &UpdateStatus::Ready {
                version: "0.3.0".into(),
                notes: "notes".into()
            }
        );
    }

    #[test]
    fn ready_is_not_redownloaded_and_check_is_silent() {
        let mut core = UpdaterCore::default();
        core.begin_download("0.3.0");
        core.ready("0.3.0", "");
        core.begin_check(); // scheduled tick while Ready: stays Ready, never shows Checking
        assert!(matches!(core.status(), UpdateStatus::Ready { .. }));
        assert!(!core.should_download("0.3.0"));
        assert!(!core.should_download("0.2.9"));
        core.found_none(t0());
        assert!(matches!(core.status(), UpdateStatus::Ready { .. }));
    }

    #[test]
    fn newer_manifest_supersedes_staged_and_failure_restores_it() {
        let mut core = UpdaterCore::default();
        core.begin_download("0.3.0");
        core.ready("0.3.0", "old");
        assert!(core.should_download("0.3.1"));
        core.begin_download("0.3.1");
        assert!(matches!(core.status(), UpdateStatus::Downloading { .. }));
        core.failed("boom", t0());
        // The 0.3.0 payload is still staged and valid — keep offering it.
        assert_eq!(
            core.status(),
            &UpdateStatus::Ready {
                version: "0.3.0".into(),
                notes: "old".into()
            }
        );
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
        core.ready("0.3.0", "");
        assert_eq!(
            core.can_install(false),
            Err(InstallRefusal::RecordingInProgress)
        );
        assert_eq!(core.can_install(true), Ok("0.3.0".to_string()));
    }

    #[test]
    fn status_serialises_with_kebab_tags() {
        let s = serde_json::to_string(&UpdateStatus::Downloading {
            version: "0.3.0".into(),
            received: 1,
            total: None,
        })
        .unwrap();
        assert!(s.contains("\"state\":\"downloading\""), "{s}");
        let s = serde_json::to_string(&UpdateStatus::Idle { last_checked: None }).unwrap();
        assert_eq!(s, r#"{"state":"idle","last_checked":null}"#);
    }
}
