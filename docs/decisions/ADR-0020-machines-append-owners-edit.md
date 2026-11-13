# ADR-0020: Automatic processes append; only the owner edits

Status: Accepted
Date: 2026-11-13

## Context

Topics (specs/0089) keep a running log for a subject across meetings. Part of the log is
written by background jobs: a timeline entry after each linked meeting is summarized, and
suggested notes when a topic comes up in a meeting that is not linked to it. The rest is the
owner's: their own notes, and any edits to an entry.

Models rewrite. If a job could update text a person can see, a summary regenerated tomorrow
would silently replace the correction they made today, and "my notes" would stop being theirs.
Trust in the log depends on edits sticking.

## Decision

Automatic processes insert entries and suggestions. They never update text a person can see
and correct. Owner actions are the only updates.

- Jobs write new rows only: a new entry, a new suggestion. A re-run (regenerated summary,
  detector run after a dismissal, a new topic added) finds the existing row and does nothing.
- The only code that assigns an entry body is owner-triggered: edit, revert to original,
  rewrite from meeting, and accepting a suggested note.
- Topic notes are assigned in exactly one place, the owner's save.
- Metadata that is not text a person edits (for example marking an entry's source meeting as
  deleted) is allowed to update; the rule protects text, not bookkeeping.

## Precedent

- specs/0034: pristine and protected rows. Regenerating never overwrites a row a person touched.
- specs/0088: protected rows in the to-do list and Inbox.

## Enforcement

A source-scan test (`only_named_fns_write_topic_entries` in `topics/ledger.rs`) reads the topics
module and fails if any function other than the named owner-triggered writers assigns
`body_json` or `ai_body_json`, or if anything other than the notes save assigns topic notes.
Adding a new writer means naming it in the test, which makes the decision visible in review.

## Consequences

- A job can never fix a bad entry; the owner uses Rewrite from meeting. That is deliberate.
- The scan matches assignments in SQL text, so it is a tripwire, not a proof; review still
  matters for any new path that touches topic tables.
