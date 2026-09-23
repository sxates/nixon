//! specs/0074 W6 spike (task 24): which process holds the microphone during a Teams or
//! Google Meet call, and does it keep holding it while muted?
//!
//! A diagnostic, not a regression test, so it is `#[ignore]`d. Run it during each scenario
//! and hand back the output:
//!
//! ```bash
//! cd frontend/src-tauri && source ~/.cargo/env
//! cargo test --features metal --test meeting_detect_spike -- --ignored --nocapture
//! # optional: NIXON_SPIKE_SECS=120 for a longer dump (default 60)
//! ```
//!
//! Scenarios: (a) a Teams call, (b) a Meet call in Chrome and in Safari, (c) Teams muted,
//! (d) Meet muted. Every second it prints each Core Audio client process's pid, bundle id and
//! whether it is running input, plus what the classifier would answer with and without a
//! calendar event. Bundle ids only: no window titles, calendar data or audio.

use std::time::Duration;

use app_lib::meeting_detect::classify::classify;
use app_lib::meeting_detect::sample::audio_processes;

#[test]
#[ignore = "diagnostic spike: run by hand during a call (specs/0074 W6)"]
fn meeting_detect_spike_dump_audio_processes() {
    let secs: u64 = std::env::var("NIXON_SPIKE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let own_pid = std::process::id() as i32;
    for tick in 0..secs {
        let mut procs = audio_processes();
        procs.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
        println!(
            "t={tick:>3}s  {} process(es); classify: no-event={:?} with-event={:?}",
            procs.len(),
            classify(false, &procs, own_pid, false, None),
            classify(false, &procs, own_pid, true, None),
        );
        for p in &procs {
            println!(
                "    pid={:<6} input={:<5} {}",
                p.pid,
                p.running_input,
                if p.bundle_id.is_empty() {
                    "<no bundle id>"
                } else {
                    &p.bundle_id
                }
            );
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
