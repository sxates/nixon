'use client';

/**
 * Copy an Ask-AI answer as prose, with the `[M#]` citation markers taken out.
 *
 * Owner feedback 2026-09-21 — "I want an easy way to copy an answer I get and be able to
 * paste it somewhere else in a clean way, without all the meeting references embedded."
 * The markers render as chips labelled with their meeting titles, which is right in a
 * window that can link to them and wrong in a message to someone who can't.
 *
 * Used by both the live answer card and every expanded history row, so "copy" means the
 * same thing wherever you find it. A text button (no glyph, §Icons); the check that
 * confirms the copy is functional, so it stays.
 */

import { useCallback, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { Check } from '@/components/ui/icons';
import { answerAsPlainProse } from '@/lib/ask-ai';

export function CopyAnswerButton({
  markdown,
  className,
}: {
  markdown: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  const onCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(answerAsPlainProse(markdown));
      setCopied(true);
      // Purely a confirmation flash — no toast for the happy path, the check says it.
      setTimeout(() => setCopied(false), 1600);
    } catch (err) {
      console.error('[Ask] Failed to copy the answer:', err);
      toast.error('Could not copy the answer');
    }
  }, [markdown]);

  return (
    <Button
      variant="outline"
      size="xs"
      onClick={() => void onCopy()}
      title="Copy the answer without its meeting references"
      aria-label="Copy answer"
      className={className}
    >
      {copied && <DeckIcon icon={Check} size={12} className="text-success" />}
      {copied ? 'Copied' : 'Copy'}
    </Button>
  );
}
