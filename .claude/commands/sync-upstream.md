---
description: Fetch upstream meetily and summarize changes worth cherry-picking
---

We track meetily as the `upstream` remote. Review what's new without merging:

1. `git fetch upstream`
2. `git log --oneline main..upstream/main | head -50` — list commits we don't have.
3. Focus on our high-value KEEP areas — summarize changes touching:
   - `frontend/src-tauri/src/audio/**`, `whisper_engine/**`, `parakeet_engine/**`
   - bug fixes to capture/mixing/VAD/transcription
   Use `git log --oneline main..upstream/main -- <path>` and `git show <sha> --stat`.
4. Report a short list: which upstream commits are worth cherry-picking into Nixon and why,
   and which to ignore (e.g. PRO/branding/legacy-backend changes, or things we've replaced).

Do NOT merge or cherry-pick automatically — just recommend. The user decides; cherry-picks
happen only on explicit request and on a feature branch.
