'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { EyeOff, Pencil, Plus, RotateCcw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { TemplateEditorDialog } from './TemplateEditorDialog';
import { DeleteTemplateDialog } from './DeleteTemplateDialog';
import { SOURCE_LABEL, TemplateListItem, TemplateSource } from './types';
import { SettingsGroup, SettingsNote, SettingsSection } from '@/components/ui/settings';

/** Badge accents per source — filled muted for built-ins, filled brand for custom,
 *  outline for edited: three distinct looks, none relying on hue alone. */
const SOURCE_BADGE_CLASS: Record<TemplateSource, string> = {
  builtin: 'bg-muted text-muted-foreground',
  custom: 'bg-brand/10 text-brand',
  override: 'border border-foreground/40 bg-transparent text-foreground',
};

/**
 * Settings → Templates (specs/0020 task 9): list every summary template with its
 * source badge, edit any of them (editing a built-in stores an override), create
 * new ones from a copy of Standard Meeting, and delete custom/edited entries
 * (deleting an edited built-in reverts it to the original).
 */
export function TemplateSettings() {
  const [templates, setTemplates] = useState<TemplateListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Editor target: undefined = closed, null = new template, string = edit that id.
  const [editorTarget, setEditorTarget] = useState<
    { templateId: string | null; source: TemplateSource | null } | undefined
  >(undefined);
  const [deleteTarget, setDeleteTarget] = useState<TemplateListItem | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<TemplateListItem[]>('api_list_templates');
      setTemplates(Array.isArray(list) ? list : []);
      setLoadError(null);
    } catch (error) {
      console.error('Failed to load templates:', error);
      setLoadError(typeof error === 'string' ? error : 'Could not load templates.');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Remove/restore (specs/0020 follow-up): shipped templates the user will never
  // use are HIDDEN, not deleted — they drop out of pickers and this main list,
  // and can be restored from the section at the bottom. Old meetings that
  // persisted a hidden template keep summarizing with it.
  const setHidden = useCallback(
    async (template: TemplateListItem, hidden: boolean) => {
      try {
        await invoke('api_set_template_hidden', { id: template.id, hidden });
        toast.success(hidden ? 'Template removed' : 'Template restored', {
          description: hidden
            ? `"${template.name}" won't appear in template lists. Restore it anytime from the Hidden section below.`
            : `"${template.name}" is back in your template lists.`,
        });
        await refresh();
      } catch (error) {
        console.error('Failed to update template visibility:', error);
        toast.error(hidden ? 'Could not remove template' : 'Could not restore template', {
          description: typeof error === 'string' ? error : undefined,
        });
      }
    },
    [refresh],
  );

  const visibleTemplates = templates.filter((t) => !t.hidden);
  const hiddenTemplates = templates.filter((t) => t.hidden);

  return (
    <div className="space-y-8">
      <SettingsSection
        title="Summary templates"
        description="Templates shape what the AI writes for each meeting — the sections, their order, and the instructions behind them. Editing a built-in keeps the original and saves your changes as an edited version."
      >
        <div className="flex justify-end">
          <Button
            variant="brand"
            size="sm"
            onClick={() => setEditorTarget({ templateId: null, source: null })}
          >
            <Plus className="h-4 w-4" />
            New template
          </Button>
        </div>

        {loadError && (
          <SettingsNote tone="warn" className="text-destructive">
            {loadError}
          </SettingsNote>
        )}

        <SettingsGroup>
          {loading && (
            <div className="py-3 u-meta">Loading templates…</div>
          )}
          {visibleTemplates.map((template) => (
          <div key={template.id} className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-b-0">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="font-medium truncate">{template.name}</span>
                <span
                  className={`flex-shrink-0 rounded-[3px] px-2 py-0.5 text-[11px] font-semibold ${SOURCE_BADGE_CLASS[template.source]}`}
                >
                  {SOURCE_LABEL[template.source]}
                </span>
              </div>
              {template.description && (
                <div className="mt-0.5 text-sm text-muted-foreground line-clamp-2">
                  {template.description}
                </div>
              )}
            </div>
            <div className="flex flex-shrink-0 items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setEditorTarget({ templateId: template.id, source: template.source })}
                aria-label={`Edit ${template.name}`}
              >
                <Pencil className="h-4 w-4" />
                Edit
              </Button>
              {/* Shipped templates hide (restorable below); Edited reverts to the
                  built-in; Custom deletes for real. */}
              {template.source === 'builtin' ? (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void setHidden(template, true)}
                  aria-label={`Remove ${template.name} from your template lists`}
                  title="Remove from your lists (restorable)"
                >
                  <EyeOff className="h-4 w-4" />
                  Remove
                </Button>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  className="text-destructive hover:text-destructive"
                  onClick={() => setDeleteTarget(template)}
                  aria-label={
                    template.source === 'override'
                      ? `Revert ${template.name} to the built-in version`
                      : `Delete ${template.name}`
                  }
                  title={template.source === 'override' ? 'Revert to built-in' : 'Delete'}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              )}
            </div>
          </div>
        ))}
        {!loading && !loadError && visibleTemplates.length === 0 && (
            <div className="py-3 u-meta">No templates found.</div>
          )}
        </SettingsGroup>
      </SettingsSection>

      {hiddenTemplates.length > 0 && (
        <SettingsSection
          title="Hidden templates"
          description="Removed from your lists — meetings that already use one keep working. Restore anytime."
        >
          <SettingsGroup>
          {hiddenTemplates.map((template) => (
            <div
              key={template.id}
              className="flex items-center justify-between gap-4 border-b border-border py-3 opacity-70 last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <span className="font-medium truncate">{template.name}</span>
                {template.description && (
                  <div className="mt-0.5 text-sm text-muted-foreground line-clamp-1">
                    {template.description}
                  </div>
                )}
              </div>
              <Button
                variant="outline"
                size="sm"
                className="flex-shrink-0"
                onClick={() => void setHidden(template, false)}
                aria-label={`Restore ${template.name}`}
              >
                <RotateCcw className="h-4 w-4" />
                Restore
              </Button>
            </div>
          ))}
          </SettingsGroup>
        </SettingsSection>
      )}

      <TemplateEditorDialog
        open={editorTarget !== undefined}
        onOpenChange={(open) => {
          if (!open) setEditorTarget(undefined);
        }}
        templateId={editorTarget?.templateId ?? null}
        source={editorTarget?.source ?? null}
        onSaved={refresh}
      />

      <DeleteTemplateDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        template={deleteTarget}
        onDeleted={refresh}
      />
    </div>
  );
}
