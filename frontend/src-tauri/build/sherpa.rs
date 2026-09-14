// ============================================================================
// sherpa-onnx dylib bundling (speaker diarization — specs/0010, ADR-0005)
// ============================================================================
//
// `sherpa-rs-sys` (with `download-binaries`) fetches a prebuilt sherpa-onnx and
// copies its dylibs into `target/<profile>/` (and `deps/`, `examples/`). The main
// app links `@rpath/libsherpa-onnx-c-api.dylib`, and that lib in turn loads
// `@rpath/libonnxruntime.1.17.1.dylib` via its own `@loader_path` rpath. At dev
// time `cargo run` resolves these through the `target/<profile>/deps` rpath, but a
// BUNDLED `.app` has no such rpath, so launch fails with:
//   dyld: Library not loaded: @rpath/libsherpa-onnx-c-api.dylib
//
// Fix (mirrors the ffmpeg/llama-helper sidecar handling): copy both dylibs into a
// stable `frameworks/` dir that `tauri.conf.json` declares under
// `bundle.macOS.frameworks` — Tauri copies those into `Contents/Frameworks/` and
// adds `@executable_path/../Frameworks` to the binary's rpath. We ALSO emit that
// rpath here via `cargo:rustc-link-arg` so the bundled binary can resolve the
// dylibs even if the bundler's rpath handling changes, and we add
// `@executable_path` as a belt-and-suspenders fallback.
//
// macOS-only; a no-op elsewhere (diarization is macOS-only like the audio stack).

#[cfg(target_os = "macos")]
const SHERPA_DYLIBS: &[&str] = &["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.17.1.dylib"];

/// Copy sherpa's runtime dylibs into `frontend/src-tauri/frameworks/` (declared in
/// tauri.conf.json) and add the bundle rpaths. Call from `build.rs` AFTER the deps
/// have been built/fetched is not possible (build scripts run before link), so we
/// copy from the well-known `target/<profile>/` location populated by
/// `sherpa-rs-sys`'s own build script, which runs before ours in the same build.
#[cfg(target_os = "macos")]
pub fn bundle_sherpa_dylibs() {
    use std::path::PathBuf;

    // rpaths so the bundled binary finds the dylibs in Contents/Frameworks/.
    // (Harmless for `cargo run`, which also resolves via target/<profile>/deps.)
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let frameworks_dir = PathBuf::from(&manifest_dir).join("frameworks");
    if let Err(e) = std::fs::create_dir_all(&frameworks_dir) {
        println!("cargo:warning=⚠️  diarization: failed to create frameworks dir: {e}");
        return;
    }

    // Locate the dylibs that sherpa-rs-sys copied into target/<profile>/.
    let Some(target_profile_dir) = sherpa_dylib_source_dir() else {
        println!(
            "cargo:warning=⚠️  diarization: could not locate target/<profile> dir to copy \
             sherpa dylibs from; bundled app may fail to launch. (cargo run still works.)"
        );
        return;
    };

    for name in SHERPA_DYLIBS {
        let src = target_profile_dir.join(name);
        let dst = frameworks_dir.join(name);

        // Prefer the freshly-built dylib; fall back to a previously-copied one so
        // a clean check of THIS crate (before deps relink) doesn't blow away a
        // valid bundle input.
        if !src.exists() {
            if dst.exists() {
                println!(
                    "cargo:warning=ℹ️  diarization: {name} not in target/<profile> yet; \
                     keeping existing frameworks/{name}"
                );
                continue;
            }
            println!(
                "cargo:warning=⚠️  diarization: {name} not found at {} and no cached copy; \
                 bundled app may fail to launch.",
                src.display()
            );
            continue;
        }

        match std::fs::copy(&src, &dst) {
            Ok(_) => println!("cargo:warning=✅ diarization: bundled {name}"),
            Err(e) => println!("cargo:warning=⚠️  diarization: failed to copy {name}: {e}"),
        }
    }
}

/// The `target/<profile>/` directory where `sherpa-rs-sys` deposits its dylibs.
/// Derived from `OUT_DIR` (`.../target/<profile>/build/<pkg>-<hash>/out`).
#[cfg(target_os = "macos")]
fn sherpa_dylib_source_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let out_dir = std::env::var("OUT_DIR").ok()?;
    let profile = std::env::var("PROFILE").ok()?;
    // Walk up from OUT_DIR until we hit the dir whose file name == PROFILE
    // (debug/release). That is target/<profile>/.
    let mut p = PathBuf::from(out_dir);
    while let Some(parent) = p.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some(profile.as_str()) {
            // The dylibs sit directly under target/<profile>/ (and deps/). Prefer
            // the top-level copy; if absent, fall back to deps/.
            let top = parent.to_path_buf();
            if top.join(SHERPA_DYLIBS[0]).exists() {
                return Some(top);
            }
            let deps = top.join("deps");
            if deps.join(SHERPA_DYLIBS[0]).exists() {
                return Some(deps);
            }
            return Some(top);
        }
        p = parent.to_path_buf();
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub fn bundle_sherpa_dylibs() {}
