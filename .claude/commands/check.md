---
description: Run the Definition-of-Done gate (cargo check + clippy + pnpm lint)
---

Run the project's quality gate and report results concisely (pass/fail per step, with the
first real error if any):

1. `cd frontend/src-tauri && cargo check`
2. `cd frontend/src-tauri && cargo clippy --all-targets -- -D warnings` (report warnings; only
   hard-fail on errors unless the user asked for strict clippy).
3. `cd frontend && pnpm lint`
4. `scripts/check-file-size.sh` (specs/0065 file-size gate — reports the excess budget and
   its remaining headroom; if it notes a file that dropped under the cap, also run it with
   `--update` and include the updated `file-size-tracked.txt`/`file-size-budget.txt` in the
   change).

If a step fails, surface the actionable error and stop before later steps only if the failure
would make them meaningless. Do not attempt fixes unless asked — just report the gate status.
