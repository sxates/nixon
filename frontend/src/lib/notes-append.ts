/**
 * The "☐ To-do: …" marker line (specs/0088 quick-add). Quick add used to append one to the
 * live meeting's My Notes; since v0.18 it doesn't (the record screen's To-Dos list shows the
 * item), but notes written before then still hold these lines. The prefix stays load-bearing:
 * the action-item extractor skips lines that start with it (the to-do already exists), so
 * they are not suggested again.
 */

export const TODO_NOTE_PREFIX = '☐ To-do:';
