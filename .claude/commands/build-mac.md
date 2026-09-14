---
description: Production Metal build of the Nixon macOS app
---

Produce a production build:

1. `cd frontend && pnpm install` (if needed).
2. `cd frontend && ./build-gpu.sh` (Metal). Use this rather than `clean_build.sh`: it builds
   the required `llama-helper` sidecar (release) before the Tauri bundle. (`clean_build.sh`
   skips the sidecar and fails on a clean tree — see `/CLAUDE.md` → Build & run.) Long-running
   — run in background and report the resulting `.app`/`.dmg` path under
   `frontend/src-tauri/target/` on success, or the first real error on failure.

Surface any missing toolchain prerequisites (Rust target, cmake/llvm) explicitly rather than
guessing around them. Ensure `~/.cargo/env` is sourced if `cargo` isn't found.
