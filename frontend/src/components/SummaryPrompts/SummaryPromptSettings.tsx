'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Trash2 } from '@/components/ui/icons';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';
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
        action={
          <Button variant="outline" size="sm" onClick={() => setEditorTarget(null)}>
            New prompt
          </Button>
        }
      >
        {loadError && <Callout tone="error">{loadError}</Callout>}

        <SettingsGroup>
          {loading && <div className="py-3 u-meta">Loading prompts…</div>}
          {prompts.map((prompt) => (
            <SettingsRow
              key={prompt.id}
              label={prompt.name}
              aside={
                prompt.is_default || prompt.extract_action_items ? (
                  <>
                    {prompt.is_default && <Badge tone="brand">Default</Badge>}
                    {prompt.extract_action_items && <Badge>Action items</Badge>}
                  </>
                ) : undefined
              }
              description={
                <span className="mt-0.5 line-clamp-2 whitespace-pre-line">{prompt.body}</span>
              }
              control={
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setEditorTarget(prompt)}
                    aria-label={`Edit ${prompt.name}`}
                  >
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
                        Set as default
                      </Button>
                      <IconButton
                        size="md"
                        label={`Delete ${prompt.name}`}
                        icon={<DeckIcon icon={Trash2} />}
                        className="text-destructive hover:text-destructive"
                        onClick={() => setDeleteTarget(prompt)}
                      />
                    </>
                  )}
                </div>
              }
            />
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
