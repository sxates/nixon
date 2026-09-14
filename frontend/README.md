# Nixon — Frontend

The Nixon desktop app: **Tauri 2 + Next.js 14 + React 18**. The Rust core lives in
`src-tauri/`, the Next.js UI in `src/`.

## Prerequisites (macOS / Apple Silicon)

- Node.js 18+
- Rust (latest stable)
- pnpm 8+
- [Xcode Command Line Tools](https://developer.apple.com/download/all/?q=xcode)

## Build & run (Metal)

From this `frontend/` directory:

```bash
pnpm install
./dev-nixon.sh   # dev build ("Dev Nixon"), isolated data — sources ~/.cargo/env first
./build-gpu.sh   # production build → ../target/release/bundle (.app + .dmg)
```

`./dev-nixon.sh` wraps the upstream `dev-gpu.sh` and puts `cargo` on PATH first
(running `dev-gpu.sh` / `build-gpu.sh` directly fails with `cargo: command not found`
unless cargo is already on PATH). Both scripts build the required `llama-helper`
sidecar before bundling — a bare `pnpm tauri:dev` will fail on a clean tree until
that sidecar exists.

## Project structure

```
frontend/
├── src/          # Next.js frontend (React/TS, BlockNote editor, Tauri IPC)
├── src-tauri/    # Rust backend (audio capture, transcription, summarization, DB)
├── public/       # Static assets
└── package.json
```

See the repo-root [`CLAUDE.md`](../CLAUDE.md) for the full architecture map, build
gotchas, and the Definition-of-Done gate (`pnpm lint`).
