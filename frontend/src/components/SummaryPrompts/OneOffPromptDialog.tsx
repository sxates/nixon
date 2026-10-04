'use client';

import { useEffect, useState } from 'react';
import { Loader2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { Label } from '@/components/ui/label';
import { MAX_PROMPT_CHARS } from '@/lib/summary-prompts';
import { PromptNotesHint } from './PromptNotesHint';

interface OneOffPromptDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Pre-fill when the meeting already has a custom prompt. */
  initialBody: string;
  initialExtract: boolean;
  /** Persist the one-off prompt. A rejection keeps the dialog open (the caller toasts). */
  onSubmit: (body: string, extract: boolean) => Promise<void>;
}

/**
 * Per-meeting one-off prompt (specs/0079 W4): free-form instructions for just this
 * meeting, plus the "Extract action items" choice.
 */
export function OneOffPromptDialog({
  open,
  onOpenChange,
  initialBody,
  initialExtract,
  onSubmit,
}: OneOffPromptDialogProps) {
  const [body, setBody] = useState(initialBody);
  const [extract, setExtract] = useState(initialExtract);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Reset each time the dialog opens.
  useEffect(() => {
    if (!open) return;
    setBody(initialBody);
    setExtract(initialExtract);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-seed on open
  }, [open]);

  const overCap = body.length > MAX_PROMPT_CHARS;
  const canSubmit = !isSubmitting && body.trim().length > 0 && !overCap;

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setIsSubmitting(true);
    try {
      await onSubmit(body, extract);
    } catch {
      // The hook already toasted; stay open so nothing typed is lost.
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isSubmitting ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Custom prompt for this meeting</DialogTitle>
          <DialogDescription>
            Tell the model how you want these notes shaped. It applies to this meeting only
            unless you choose to keep it afterwards.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="one-off-prompt-body">Prompt</Label>
            <Textarea
              id="one-off-prompt-body"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              placeholder="e.g. Focus on decisions and open questions."
              rows={8}
              aria-describedby="one-off-prompt-notes-hint"
            />
            <div className="flex items-start justify-between gap-3">
              <PromptNotesHint id="one-off-prompt-notes-hint" />
              <div
                className={`shrink-0 text-right text-xs ${overCap ? 'text-destructive' : 'text-muted-foreground'}`}
              >
                {`${body.length} / ${MAX_PROMPT_CHARS}`}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <input
              id="one-off-prompt-extract"
              type="checkbox"
              checked={extract}
              onChange={(e) => setExtract(e.target.checked)}
              className="h-4 w-4 rounded border-input accent-brand"
            />
            <Label htmlFor="one-off-prompt-extract">Extract action items</Label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isSubmitting}>
            Cancel
          </Button>
          <Button variant="brand" onClick={handleSubmit} disabled={!canSubmit}>
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Use for this meeting
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
