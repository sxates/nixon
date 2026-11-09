//! specs/0086 — which update feed a check polls. Pure, no Tauri.
//!
//! The everyone feed lives in `tauri.conf.json` (`plugins.updater.endpoints`) and stays
//! the single source of truth for it: with early access off the driver leaves the
//! plugin's builder alone. With it on, the driver points the builder at [`EARLY_FEED`],
//! the same host with a second manifest path (ADR-0012).
//!
//! Turning early access off never downgrades: the plugin only offers versions strictly
//! newer than the running one, so an installed early build simply waits until the
//! everyone feed passes it.

use url::Url;

/// The early access manifest. Same templating as the config endpoint; the plugin fills
/// in `{{current_version}}` and `{{target}}` whether or not they were percent-encoded.
pub const EARLY_FEED: &str =
    "https://nixonapp.com/updates/early.json?current_version={{current_version}}&target={{target}}";

/// None = use the endpoint from tauri.conf.json (the everyone feed).
pub fn endpoint_override(early_access: bool) -> Option<&'static str> {
    early_access.then_some(EARLY_FEED)
}

/// The endpoint list to hand `UpdaterBuilder::endpoints`, or None to keep the config's.
pub fn endpoints(early_access: bool) -> Result<Option<Vec<Url>>, url::ParseError> {
    endpoint_override(early_access)
        .map(|raw| Url::parse(raw).map(|url| vec![url]))
        .transpose()
}

/// The late-flip discard. A check that polled the early feed must not stage what it
/// downloaded if early access was turned off while it ran (DECIDED-1: a machine that
/// opted out should not install an early build at its next restart). `opt_outs_*` is
/// the count of switch-offs, read at the start of the check and again just before
/// staging.
pub fn discard_late_flip(used_early: bool, opt_outs_at_start: u64, opt_outs_now: u64) -> bool {
    used_early && opt_outs_now != opt_outs_at_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_keeps_the_config_endpoint() {
        assert_eq!(endpoint_override(false), None);
        assert_eq!(endpoints(false).unwrap(), None);
    }

    #[test]
    fn on_selects_the_early_feed_and_it_parses() {
        assert_eq!(endpoint_override(true), Some(EARLY_FEED));
        let urls = endpoints(true).unwrap().expect("early access overrides");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].host_str(), Some("nixonapp.com"));
        assert_eq!(urls[0].path(), "/updates/early.json");
        Url::parse(endpoint_override(true).unwrap()).expect("EARLY_FEED is a valid URL");
    }

    /// Pins EARLY_FEED against drift: it must be the configured everyone feed with only
    /// the manifest name changed, so host, scheme and template parameters stay in step.
    #[test]
    fn early_feed_is_the_config_endpoint_with_early_json() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let configured = conf["plugins"]["updater"]["endpoints"][0]
            .as_str()
            .expect("tauri.conf.json has an updater endpoint");
        assert!(configured.contains("/updates/latest.json"));
        assert_eq!(
            EARLY_FEED,
            configured.replace("/updates/latest.json", "/updates/early.json")
        );
    }

    #[test]
    fn late_flip_discards_only_an_early_check_after_a_switch_off() {
        assert!(!discard_late_flip(true, 3, 3), "nothing changed");
        assert!(discard_late_flip(true, 3, 4), "switched off mid-check");
        assert!(
            !discard_late_flip(false, 3, 4),
            "an everyone-feed check is never discarded by switching early access off"
        );
    }
}
