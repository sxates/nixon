'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AlertTriangle, Loader2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
  MAX_PROMPT_CHARS,
  MAX_PROMPT_NAME_CHARS,
  type SummaryPrompt,
} from '@/lib/summary-prompts';
import { PromptNotesHint } from './PromptNotesHint';

interface PromptEditorDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Prompt to edit, or `null` to create a new one. */
  prompt: SummaryPrompt | null;
  /** Called after a successful save so the caller can refresh its list. */
  onSaved: () => void | Promise<void>;
}

/**
 * Summary prompt editor (specs/0079 W4): name, free-form body with a live character
 * counter, and the "Extract action items" toggle (ON for new prompts). Backend
 * validation errors arrive as user-readable strings and are shown inline.
 */
export function PromptEditorDialog({ open, onOpenChange, prompt, onSaved }: PromptEditorDialogProps) {
  const isNew = prompt === null;
  const [name, setName] = useState('');
  const [body, setBody] = useState('');
  const [extractActionItems, setExtractActionItems] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Reset the form each time the dialog opens (or its target changes).
  useEffect(() => {
    if (!open) return;
    setName(prompt?.name ?? '');
    setBody(prompt?.body ?? '');
    setExtractActionItems(prompt?.extract_action_items ?? true);
    setSaveError(null);
  }, [open, prompt]);

  const overCap = body.length > MAX_PROMPT_CHARS;
  const canSave = !isSaving && name.trim().length > 0 && body.trim().length > 0 && !overCap;

  const handleSave = async () => {
    if (!canSave) return;
    setIsSaving(true);
    setSaveError(null);
    try {
      await invoke('api_save_summary_prompt', {
        id: prompt?.id ?? null,
        name: name.trim(),
        body,
        extractActionItems,
      });
      toast.success(isNew ? 'Prompt created' : 'Prompt saved');
      onOpenChange(false);
      await onSaved();
    } catch (error) {
      console.error('Failed to save summary prompt:', error);
      setSaveError(
        typeof error === 'string' && error
          ? error
          : error instanceof Error && error.message
            ? error.message
            : 'Could not save the prompt.',
      );
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isSaving ? undefined : onOpenChange(next))}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>{isNew ? 'New prompt' : `Edit "${prompt.name}"`}</DialogTitle>
          <DialogDescription>
            Tell the model how you want your notes shaped. Nixon always keeps its own rules: it only
            uses what was said, and it begins every summary with a title. Your prompt can&apos;t
            change those.
          </DialogDescription>
        </DialogHeader>

        <div className="-mr-2 flex-1 space-y-4 overflow-y-auto pr-2">
          <div className="space-y-1.5">
            <Label htmlFor="summary-prompt-name">Name</Label>
            <Input
              id="summary-prompt-name"
              value={name}
              maxLength={MAX_PROMPT_NAME_CHARS}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Weekly 1:1"
            />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="summary-prompt-body">Prompt</Label>
            <Textarea
              id="summary-prompt-body"
              value={body}
              onChange={(e) => setBody(e.target.value)}
              placeholder="e.g. Write short bullet notes grouped by topic. Call out decisions."
              rows={10}
              aria-describedby="summary-prompt-notes-hint"
            />
            <div className="flex items-start justify-between gap-3">
              <PromptNotesHint id="summary-prompt-notes-hint" />
              <div
                className={`shrink-0 text-right text-xs ${overCap ? 'text-destructive' : 'text-muted-foreground'}`}
              >
                {`${body.length} / ${MAX_PROMPT_CHARS}`}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <Switch
              id="summary-prompt-actions"
              checked={extractActionItems}
              onCheckedChange={setExtractActionItems}
            />
            <Label htmlFor="summary-prompt-actions">Extract action items</Label>
          </div>

          {saveError && (
            <div
              role="alert"
              className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            >
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
              {saveError}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isSaving}>
            Cancel
          </Button>
          <Button variant="brand" onClick={handleSave} disabled={!canSave}>
            {isSaving && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isNew ? 'Create prompt' : 'Save'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
