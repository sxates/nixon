use log::warn as log_warn;

pub fn format_timestamp(seconds: f64) -> String {
    let total_seconds = seconds as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, secs)
}

/// Opens macOS System Settings to a specific privacy preference pane
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn open_system_settings(preference_pane: String) -> Result<(), String> {
    use std::process::Command;

    // Construct the URL for System Settings
    let url = format!(
        "x-apple.systempreferences:com.apple.preference.security?{}",
        preference_pane
    );

    // Use the 'open' command on macOS to open the URL
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("Failed to open system settings: {}", e))?;

    Ok(())
}

/// specs/0028 hardening + specs/0029 WS1.1: the schemes `open_external_url` may hand
/// to the OS opener. Web + mail, plus the Zoom desktop-client deep-link schemes
/// (`zoommtg`/`zoomus`) that "Join & Record" launches. Everything else (file://,
/// custom app handlers, shell-interpretable strings) is refused — asking the OS to
/// `open` arbitrary schemes is a command/handler-injection risk.
fn external_url_scheme_allowed(url: &str) -> bool {
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        scheme.as_str(),
        "http" | "https" | "mailto" | "zoommtg" | "zoomus"
    )
}

#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), String> {
    use std::process::Command;

    if !external_url_scheme_allowed(&url) {
        log_warn!(
            "Refusing to open external URL with disallowed scheme: {}",
            url
        );
        return Err("Only http, https, mailto, and Zoom links can be opened".to_string());
    }

    let result = if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", &url]).output()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&url).output()
    } else {
        // Linux and other Unix-like systems
        Command::new("xdg-open").arg(&url).output()
    };

    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Failed to open URL: {}", e)),
    }
}

/// specs/0029 WS1.1 — the `open_external_url` scheme allowlist (0028 hardening) must
/// admit the Zoom desktop deep-link schemes that Join & Record launches, and keep
/// refusing everything else.
#[cfg(test)]
mod open_external_url_tests {
    use super::external_url_scheme_allowed;

    #[test]
    fn allows_web_mail_and_zoom_schemes() {
        assert!(external_url_scheme_allowed("http://example.com"));
        assert!(external_url_scheme_allowed("https://zoom.us/j/123?pwd=abc"));
        assert!(external_url_scheme_allowed("mailto:someone@example.com"));
        assert!(external_url_scheme_allowed(
            "zoommtg://zoom.us/join?confno=123"
        ));
        assert!(external_url_scheme_allowed(
            "zoomus://zoom.us/join?confno=123"
        ));
        // Scheme matching is case-insensitive.
        assert!(external_url_scheme_allowed(
            "ZOOMMTG://zoom.us/join?confno=123"
        ));
        assert!(external_url_scheme_allowed("HTTPS://example.com"));
    }

    #[test]
    fn rejects_disallowed_and_malformed_schemes() {
        assert!(!external_url_scheme_allowed("file:///etc/passwd"));
        assert!(!external_url_scheme_allowed("javascript:alert(1)"));
        assert!(!external_url_scheme_allowed("ssh://host"));
        assert!(!external_url_scheme_allowed("zoomphonecall://+15551234567"));
        assert!(!external_url_scheme_allowed("no-scheme-at-all"));
        assert!(!external_url_scheme_allowed(""));
    }
}
