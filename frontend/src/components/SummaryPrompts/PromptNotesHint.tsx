/**
 * Helper text shown under the prompt textarea in both prompt dialogs (library editor and
 * one-off). The meeting's own notes are always passed to the model alongside the prompt;
 * this tells the user they can steer how those notes are used by naming them.
 *
 * The examples are things the summary rules allow: notes shape content and emphasis, the
 * prompt shapes structure (it cannot switch the notes off reliably, so none of the examples
 * try to).
 */
export const PROMPT_NOTES_HINT =
  'Your notes and agenda are included automatically. Refer to them as "my notes" and "my agenda" to direct how they\'re used, for example "put my notes at the very top" or "list anything from my agenda that wasn\'t discussed".';

export function PromptNotesHint({ id }: { id: string }) {
  return (
    <p id={id} className="min-w-0 flex-1 text-xs text-muted-foreground">
      {PROMPT_NOTES_HINT}
    </p>
  );
}
