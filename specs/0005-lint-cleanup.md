# 0005 — ESLint backlog cleanup (inherited upstream debt)

- **Status:** Mostly done (2026-06-23). 3 of 4 rules restored to `error`
  (`no-unused-vars`, `react/no-unescaped-entities`, `prefer-const` — all at 0).
  `no-explicit-any` left at `warn` (86 remain; see below). `pnpm lint` exits 0.
- **Owner agent(s):** frontend-engineer
- **Roadmap phase:** Phase 1 (tech-debt)

## Context / Problem
The ESLint toolchain was wired up on 2026-06-23 (commit `6ba7269`: added
`eslint@8.57` + `eslint-config-next@14.2.25`, replaced the non-functional flat
`eslint.config.mjs` with `.eslintrc.json`). `pnpm lint` had never run in this fork —
meetily shipped a flat config but never installed eslint, and Next 14's `next lint`
doesn't read flat config.

Running it for the first time surfaced **~267 pre-existing errors**, all inherited from
upstream meetily — **none from spec 0003 / Vinyl work** (our new files are clean). To keep
the DoD `/check` gate usable in the meantime, these three rules were temporarily downgraded
from `error` to `warn` in `frontend/.eslintrc.json`:

| Rule | Count | Notes |
|---|---|---|
| `@typescript-eslint/no-unused-vars` | 113 | mostly trivial; largely `eslint --fix`-able |
| `@typescript-eslint/no-explicit-any` | 92 | needs real typing (services, analytics, types, hooks) |
| `react/no-unescaped-entities` | 62 | mechanical (`'` → `&apos;` etc.); `--fix`-able |
| `prefer-const` | 2 | trivial; `--fix`-able |

## Goal
Burn the backlog down to zero, then **flip the three rules back to `error`** in
`.eslintrc.json` so regressions are caught and `/check` enforces a clean lint.

## Likely direction
1. `cd frontend && npx eslint . --fix` to clear the auto-fixable unused-vars + unescaped
   entities; review the diff (don't let `--fix` delete genuinely-needed code).
2. Hand-fix the residual `no-explicit-any` with real types — concentrated in
   `src/lib/analytics.ts`, `src/services/`, `src/types/index.ts`, and the recording hooks.
3. Re-run `pnpm lint`; once clean, set the three rules back to `error`.
4. Good candidate for a multi-agent workflow (fan out per-file, isolated worktrees) given
   the file count — see the `ultracode`/Workflow option.

## Acceptance criteria
`pnpm lint` exits 0 with the three rules restored to `error`; no behavior change (pure
type/lint hygiene).

## Outcome (2026-06-23)
Done category-by-category, verifying with `pnpm lint` + `tsc --noEmit` continuously.

| Rule | Before | After | Action |
|---|---|---|---|
| `prefer-const` | 2 | **0 → `error`** | `eslint --fix` (2 trivial `let`→`const`) |
| `react/no-unescaped-entities` | 61 | **0 → `error`** | hand-escaped JSX text (`"`→`&quot;`, `'`→`&apos;`) across ~13 files |
| `@typescript-eslint/no-unused-vars` | 113 | **0 → `error`** | removed dead imports/vars/fns; `_`-prefixed intentionally-unused args/destructures/catch-bindings. Config now ignores `^_` (`args/vars/caughtErrors/destructuredArray IgnorePattern`). |
| `@typescript-eslint/no-explicit-any` | 92 | **86 → stays `warn`** | fixed 6 (the 3 form-component `Control<any>`→`Control<FieldValues>`, +3 incidental). |

### Why `no-explicit-any` stays `warn` (86 remaining)
Per the "don't guess a wrong type" rule, the residual `any` are all cases where a safe
concrete type is non-obvious or would risk a behavior/type regression:
- **`catch (err: any)`** clauses that immediately do `err?.message` — switching to `unknown`
  needs a narrow/cast (logic change), so left as-is.
- **Dynamic JSON store records** (`lib/analytics.ts` `Record<string, any>` for
  `features_used`/`copy_counts`, `services/*` `[key: string]: any`, `types/index.ts` legacy
  index signatures) — genuinely heterogeneous shapes; `unknown` would break dynamic property
  access at every call site.
- **`(t as any).field` casts** (`useTranscriptRecovery.ts`) — removing them touches data flow.

These need real per-site typing work (a follow-up), not a mechanical flip.

## Verification (final)
`cd frontend && pnpm lint` → **exit 0**, 0 errors, 122 warnings (86 `no-explicit-any` +
36 pre-existing `react-hooks/exhaustive-deps`, both out of this spec's scope).
`npx tsc --noEmit` → only the pre-existing `bun:test` error in `tests/` (no NEW errors).
No runtime behavior changed (pure hygiene).
