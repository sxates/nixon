// Zoom join-URL extraction from free-form calendar event text (specs/0008 P2).
//
// EventKit gives us the Zoom link only as plain text buried in an event's
// `url`, `location`, or `notes` (e.g. "Join Zoom Meeting\nhttps://example.zoom.us/
// j/85512345678?pwd=abcDEF.1"). We scan those fields with a single regex and
// return the first match. Kept dependency-free of EventKit so it is unit-testable.

use once_cell::sync::Lazy;
use regex::Regex;

/// Matches a Zoom join URL: `https://<sub>.zoom.us/j/<digits>` optionally
/// followed by a `?pwd=<token>` query (and we keep any trailing query string so
/// the passcode survives). Also accepts the bare `zoom.us` host (no subdomain)
/// and the `/w/` web-client variant some tenants use.
///
/// We deliberately stop the match at the first whitespace or a closing `>`/`)`
/// /quote so a link pasted inside HTML or parentheses in `notes` is clean.
static ZOOM_URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)https://[a-z0-9.\-]*\bzoom\.us/(?:j|w|s|my)/[^\s"'<>)\]]+"#)
        .expect("zoom url regex is valid")
});

/// Scan the given candidate strings (in priority order) for a Zoom join URL and
/// return the first one found, or `None`. Pass an event's `url`, then
/// `location`, then `notes` — the first non-empty match wins.
pub fn extract_zoom_url<'a>(candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    for text in candidates {
        if let Some(m) = ZOOM_URL_RE.find(text) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_join_url() {
        let url = "https://example.zoom.us/j/85512345678";
        assert_eq!(extract_zoom_url([url]).as_deref(), Some(url));
    }

    #[test]
    fn extracts_url_with_pwd_query() {
        let url = "https://us02web.zoom.us/j/81234567890?pwd=Ab1Cd2Ef3.gHiJ";
        assert_eq!(extract_zoom_url([url]).as_deref(), Some(url));
    }

    #[test]
    fn extracts_from_notes_blob() {
        let notes = "Hi team,\nJoin Zoom Meeting\nhttps://company.zoom.us/j/99988877766?pwd=Zm9vYmFy please be on time";
        assert_eq!(
            extract_zoom_url([notes]).as_deref(),
            Some("https://company.zoom.us/j/99988877766?pwd=Zm9vYmFy")
        );
    }

    #[test]
    fn strips_trailing_html_and_parens() {
        let html = r#"<a href="https://example.zoom.us/j/85512345678?pwd=secret">Join</a>"#;
        assert_eq!(
            extract_zoom_url([html]).as_deref(),
            Some("https://example.zoom.us/j/85512345678?pwd=secret")
        );

        let paren = "(https://example.zoom.us/j/12345)";
        assert_eq!(
            extract_zoom_url([paren]).as_deref(),
            Some("https://example.zoom.us/j/12345")
        );
    }

    #[test]
    fn matches_bare_host_and_web_client() {
        assert_eq!(
            extract_zoom_url(["https://zoom.us/j/12345"]).as_deref(),
            Some("https://zoom.us/j/12345")
        );
        assert_eq!(
            extract_zoom_url(["join here https://x.zoom.us/w/12345?tk=abc end"]).as_deref(),
            Some("https://x.zoom.us/w/12345?tk=abc")
        );
    }

    #[test]
    fn case_insensitive_host() {
        assert_eq!(
            extract_zoom_url(["HTTPS://Example.ZOOM.US/j/85512345678"]).as_deref(),
            Some("HTTPS://Example.ZOOM.US/j/85512345678")
        );
    }

    #[test]
    fn priority_order_first_non_empty_wins() {
        // Simulating url="", location with a link, notes with a different link:
        // location should win because it comes first and matches.
        let got = extract_zoom_url(["", "https://a.zoom.us/j/111", "https://b.zoom.us/j/222"]);
        assert_eq!(got.as_deref(), Some("https://a.zoom.us/j/111"));
    }

    #[test]
    fn returns_none_for_non_zoom() {
        assert_eq!(
            extract_zoom_url(["https://meet.google.com/abc-defg-hij"]),
            None
        );
        assert_eq!(extract_zoom_url(["Conference Room B, 4th floor"]), None);
        assert_eq!(extract_zoom_url([""]), None);
    }

    #[test]
    fn does_not_match_zoom_marketing_pages() {
        // Only /j/, /w/, /s/, /my/ paths are join links; a plain zoom.us link
        // (e.g. a profile or support page) must not be treated as a join URL.
        assert_eq!(extract_zoom_url(["https://zoom.us/download"]), None);
        assert_eq!(extract_zoom_url(["https://support.zoom.us/hc/en-us"]), None);
    }
}
