---
description: Scaffold a new numbered feature spec from the template
argument-hint: <feature name / short description>
---

Create a new spec for: **$ARGUMENTS**

Steps:
1. Look at `private/specs/` (gitignored design records) and find the highest existing `NNNN-*.md` number; the new spec is the
   next zero-padded number.
2. Copy `private/specs/TEMPLATE.md` to `private/specs/NNNN-<kebab-case-slug>.md`.
3. Fill in what you can confidently infer from `$ARGUMENTS`, `/CLAUDE.md`, `ROADMAP.md`, and
   a quick grep of the relevant code. Leave clearly-marked `TODO:` placeholders where you
   need decisions from the user.
4. For non-trivial features, prefer delegating the deep design to the `spec-architect` agent.
5. Report the new file path and the open questions that need the user's input.

Do not implement feature code — this command only produces the spec.
