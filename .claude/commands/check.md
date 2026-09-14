---
description: Run the Definition-of-Done gate (cargo check + clippy + pnpm lint)
---

Run the project's quality gate and report results concisely (pass/fail per step, with the
first real error if any):

1. `cd frontend/src-tauri && cargo check`
2. `cd frontend/src-tauri && cargo clippy --all-targets -- -D warnings` (report warnings; only
   hard-fail on errors unless the user asked for strict clippy).
3. `cd frontend && pnpm lint`
4. `scripts/check-file-size.sh` (specs/0042 file-size ratchet; if it reports a file that
   shrank, also run it with `--update` and include the tightened allowlist in the change).

If a step fails, surface the actionable error and stop before later steps only if the failure
would make them meaningless. Do not attempt fixes unless asked — just report the gate status.
