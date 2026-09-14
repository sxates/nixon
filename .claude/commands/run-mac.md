---
description: Launch the Nixon dev app on macOS (Metal)
---

Launch the app in dev mode so the user can exercise it:

1. Ensure deps are installed: `cd frontend && pnpm install` (skip if already done this session).
2. Run `cd frontend && ./dev-nixon.sh`. This is the **correct macOS dev launcher**: it ensures
   cargo is on PATH, then calls upstream's `dev-gpu.sh`, which builds the `llama-helper`
   sidecar (required — see below), auto-detects the GPU (Metal/CoreML on Apple Silicon), and
   starts the Tauri dev app + Next.js dev server. It's long-running — run it in the background
   and tail output; report when the window is up or if it fails. (Don't call `dev-gpu.sh`
   directly — it runs under non-interactive bash and fails with `cargo: command not found`.)

**Why not `clean_run.sh` / `pnpm run tauri:dev`?** Those do NOT build the `llama-helper`
sidecar and will fail on a clean tree with `resource path 'binaries/llama-helper-...' doesn't
exist`. They only work *after* the sidecar has been built once (it lives in
`frontend/src-tauri/binaries/llama-helper-<triple>`). `dev-gpu.sh` always rebuilds it.

Reminders for the user (state them, don't try to do them yourself):
- Grant **Microphone** and **Screen Recording** permissions when prompted (Screen Recording
  is required for system/Zoom audio capture; no BlackHole needed).
- First run compiles Rust + may download a Whisper model, so it can take a while.
- Ensure `~/.cargo/env` is sourced if `cargo` isn't found.
