//! specs/0084 W1 — a debug-only way to reproduce the native crash the crash-loop breaker
//! exists for. With `NIXON_DIARIZATION_TEST_CRASH=abort` (or `segv`) set, the offline pass
//! kills the process just before compute, the way a sherpa-onnx abort or segfault does: no
//! Rust unwinding, no outcome hook. Compiled into debug builds only.

/// What the env var asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Crash {
    Abort,
    Segv,
}

fn requested(value: Option<&str>) -> Option<Crash> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "abort" => Some(Crash::Abort),
        "segv" => Some(Crash::Segv),
        _ => None,
    }
}

/// Crash the process when `NIXON_DIARIZATION_TEST_CRASH` asks for it; otherwise nothing.
pub fn maybe_crash() {
    let value = std::env::var("NIXON_DIARIZATION_TEST_CRASH").ok();
    match requested(value.as_deref()) {
        Some(Crash::Abort) => {
            log::warn!("NIXON_DIARIZATION_TEST_CRASH=abort: aborting before compute");
            std::process::abort();
        }
        Some(Crash::Segv) => {
            log::warn!("NIXON_DIARIZATION_TEST_CRASH=segv: writing through a null pointer");
            // SAFETY: not safe at all, on purpose: this is the simulated native segfault.
            unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1) };
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_documented_values_crash() {
        assert_eq!(requested(Some("abort")), Some(Crash::Abort));
        assert_eq!(requested(Some(" SEGV ")), Some(Crash::Segv));
        for v in [None, Some(""), Some("1"), Some("hang")] {
            assert_eq!(requested(v), None, "{v:?}");
        }
    }
}
