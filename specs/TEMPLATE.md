# NNNN — <Feature title>

- **Status:** Draft | Approved | In progress | Done
- **Owner agent(s):** <e.g. frontend-engineer + llm-pipeline-engineer>
- **Roadmap phase:** <e.g. Phase 2>

## Context / Problem
What need does this address? What's broken or missing today? What prompted it?

## Goals
- ...

## Non-goals
- ... (explicitly out of scope, to keep the change focused)

## Approach
The chosen approach in a few sentences, and *why* over the alternatives.

## Design
### Data model
Schema/migration changes (sqlx migrations under `frontend/src-tauri/migrations/`), or
reuse of existing tables (e.g. `meeting_notes`). Name the tables/columns.

### Tauri IPC
New `#[tauri::command]`s and events (registered in `frontend/src-tauri/src/registry.rs`).

### UI
Components/routes affected under `frontend/src/`.

## Tasks
1. [ ] ... (name the real files/modules; assign an owner agent)
2. [ ] ...

## Acceptance criteria
Testable conditions; tie back to the Definition of Done in `/CLAUDE.md`.

## Risks / open questions
- ...

## Verification
How we prove it works end-to-end (commands to run, manual smoke steps, fixtures).
