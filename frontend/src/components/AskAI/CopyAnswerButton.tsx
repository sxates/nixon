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
 * same thing wherever you find it.
 */

import { useCallback, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { toast } from 'sonner';
import { answerAsPlainProse } from '@/lib/ask-ai';
import { cn } from '@/lib/utils';

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
      // Purely a confirmation flash — no toast for the happy path, the icon says it.
      setTimeout(() => setCopied(false), 1600);
    } catch (err) {
      console.error('[Ask] Failed to copy the answer:', err);
      toast.error('Could not copy the answer');
    }
  }, [markdown]);

  return (
    <button
      type="button"
      onClick={() => void onCopy()}
      title="Copy the answer without its meeting references"
      aria-label="Copy answer"
      className={cn(
        'inline-flex h-7 flex-shrink-0 items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 text-xs font-semibold text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        className,
      )}
    >
      {copied ? (
        <Check size={13} aria-hidden="true" className="text-success" />
      ) : (
        <Copy size={13} aria-hidden="true" />
      )}
      {copied ? 'Copied' : 'Copy'}
    </button>
  );
}
