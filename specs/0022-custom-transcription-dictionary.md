# 0022 — Custom transcription dictionary

- **Status:** Draft
- **Owner agent(s):** audio-engineer + rust-core-engineer + frontend-engineer
- **Roadmap phase:** Post-1.0 (graduated from `specs/0019` WS7)

## Context / Problem

From 1.0 testing (note 18 in `specs/0019`): common domain terms, product names, and people's
names are frequently mis-transcribed (e.g. "Vinyl" → "vinal"), with no way to correct them.
There is currently no user-managed dictionary or biasing mechanism in the STT path
(`frontend/src-tauri/src/whisper_engine/`, `parakeet_engine/`, `audio/transcription/`).

## Goals

- Let the user maintain a dictionary of terms (and optional "sounds like"/replacement mappings).
- Apply it so those terms are transcribed correctly, both live and in the final transcript.

## Non-goals

- Per-meeting dictionaries (start global; revisit if needed).
- Fine-tuning or retraining models.

## Approach

Two complementary mechanisms, smallest-first:
1. **Post-STT correction pass** (model-agnostic, works for Whisper and Parakeet): apply
   user-defined replacements / fuzzy corrections to recognized text before persistence. This is
   the reliable baseline.
2. **Biasing where supported:** feed dictionary terms into Whisper's `initial_prompt` (context
   biasing) to improve recognition at the source. Parakeet biasing support is an open question.

Apply correction in the transcription path so both live segments and the stored transcript
benefit; keep the original raw text if we want reversibility.

## Design

### Data model
- Migration under `frontend/src-tauri/migrations/`: a `dictionary_terms` table
  (`term`, optional `replacement`/`aliases`, `enabled`, timestamps). Global scope.

### Tauri IPC
- `api_list_dictionary_terms`, `api_add_dictionary_term`, `api_update_dictionary_term`,
  `api_delete_dictionary_term` (new module; register in `lib.rs`).
- Load terms into the transcription engine config at recording start (and on change).

### Transcription path
- A correction step in `audio/transcription/` applied to recognized segments before they're
  emitted/persisted; Whisper `initial_prompt` biasing in `whisper_engine/`.
- Live segments (`diarization/live` consumers / `TranscriptContext`) display the corrected text.

### UI
- Settings surface (under `frontend/src/app/settings` or `SettingsModal`) to manage terms.

## Tasks
1. [ ] `dictionary_terms` migration + repo (rust-core-engineer).
2. [ ] CRUD Tauri commands + register (rust-core-engineer).
3. [ ] Post-STT correction pass in the transcription path (audio-engineer).
4. [ ] Whisper `initial_prompt` biasing from dictionary terms; assess Parakeet (audio-engineer).
5. [ ] Settings UI to manage the dictionary (frontend-engineer).

## Acceptance criteria
- A user can add a term; subsequent transcription (live + final) renders it correctly.
- Corrections apply without a noticeable latency regression in the live path.
- DoD per `/CLAUDE.md` (incl. the `cargo test` transcription fixtures in `tests/`).

## Risks / open questions
- Over-correction (false replacements on similar words) — scope to word-boundary / confidence
  thresholds; let users disable a term.
- Whisper `initial_prompt` length limits; Parakeet biasing support unknown.
- Whether to keep raw + corrected text (reversibility) vs. correct in place.

## Verification
`cargo test --features metal --test transcription_engine` extended with a dictionary-correction
fixture (generate audio via macOS `say` for a known mis-transcribed term); manual smoke confirming
a configured term is corrected live.
