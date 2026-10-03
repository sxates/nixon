'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Pencil, Plus, Star, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { SettingsGroup, SettingsNote, SettingsSection } from '@/components/ui/settings';
import type { SummaryPrompt } from '@/lib/summary-prompts';
import { PromptEditorDialog } from './PromptEditorDialog';
import { DeletePromptDialog } from './DeletePromptDialog';

/**
 * Settings → Summary prompts (specs/0079 W4): the saved-prompt library that replaced
 * summary templates. List, create, edit, set the default, and delete.
 */
export function SummaryPromptSettings() {
  const [prompts, setPrompts] = useState<SummaryPrompt[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Editor target: undefined = closed, null = new prompt, otherwise edit that prompt.
  const [editorTarget, setEditorTarget] = useState<SummaryPrompt | null | undefined>(undefined);
  const [deleteTarget, setDeleteTarget] = useState<SummaryPrompt | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<SummaryPrompt[]>('api_list_summary_prompts');
      setPrompts(Array.isArray(list) ? list : []);
      setLoadError(null);
    } catch (error) {
      console.error('Failed to load summary prompts:', error);
      setLoadError(typeof error === 'string' ? error : 'Could not load prompts.');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const setDefault = useCallback(
    async (prompt: SummaryPrompt) => {
      try {
        await invoke('api_set_default_summary_prompt', { id: prompt.id });
        toast.success('Default prompt updated', {
          description: `"${prompt.name}" is now the default.`,
        });
        await refresh();
      } catch (error) {
        console.error('Failed to set default prompt:', error);
        toast.error('Could not set default prompt', {
          description: typeof error === 'string' ? error : undefined,
        });
      }
    },
    [refresh],
  );

  return (
    <div className="space-y-8">
      <SettingsSection
        title="Summary prompts"
        description="A prompt tells the model how you want your meeting notes shaped. The default is used for new meetings; you can pick a different one per meeting."
      >
        <div className="flex justify-end">
          <Button variant="brand" size="sm" onClick={() => setEditorTarget(null)}>
            <Plus className="h-4 w-4" />
            New prompt
          </Button>
        </div>

        {loadError && (
          <SettingsNote tone="warn" className="text-destructive">
            {loadError}
          </SettingsNote>
        )}

        <SettingsGroup>
          {loading && <div className="py-3 u-meta">Loading prompts…</div>}
          {prompts.map((prompt) => (
            <div
              key={prompt.id}
              className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate font-medium">{prompt.name}</span>
                  {prompt.is_default && (
                    <span className="flex-shrink-0 rounded-[3px] bg-brand/10 px-2 py-0.5 text-[11px] font-semibold text-brand">
                      Default
                    </span>
                  )}
                  {prompt.extract_action_items && (
                    <span className="flex-shrink-0 rounded-[3px] bg-muted px-2 py-0.5 text-[11px] font-semibold text-muted-foreground">
                      Action items
                    </span>
                  )}
                </div>
                <div className="mt-0.5 line-clamp-2 whitespace-pre-line text-sm text-muted-foreground">
                  {prompt.body}
                </div>
              </div>
              <div className="flex flex-shrink-0 items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setEditorTarget(prompt)}
                  aria-label={`Edit ${prompt.name}`}
                >
                  <Pencil className="h-4 w-4" />
                  Edit
                </Button>
                {!prompt.is_default && (
                  <>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void setDefault(prompt)}
                      aria-label={`Set ${prompt.name} as default`}
                    >
                      <Star className="h-4 w-4" />
                      Set as default
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="text-destructive hover:text-destructive"
                      onClick={() => setDeleteTarget(prompt)}
                      aria-label={`Delete ${prompt.name}`}
                      title="Delete"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </>
                )}
              </div>
            </div>
          ))}
          {!loading && !loadError && prompts.length === 0 && (
            <div className="py-3 u-meta">
              No prompts yet. Create one to shape how your meeting notes are written.
            </div>
          )}
        </SettingsGroup>
      </SettingsSection>

      <PromptEditorDialog
        open={editorTarget !== undefined}
        onOpenChange={(open) => {
          if (!open) setEditorTarget(undefined);
        }}
        prompt={editorTarget ?? null}
        onSaved={refresh}
      />

      <DeletePromptDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        prompt={deleteTarget}
        onDeleted={refresh}
      />
    </div>
  );
}
