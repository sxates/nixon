'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AlertTriangle, ChevronDown, ChevronUp, Loader2, Plus, Trash2 } from 'lucide-react';
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
import {
  SECTION_FORMATS,
  SectionFormat,
  TemplateDetails,
  TemplateSection,
  TemplateSource,
} from './types';

/** Seed for "New template": a copy of this built-in's details (specs/0020 task 9). */
const NEW_TEMPLATE_SEED_ID = 'standard_meeting';

interface TemplateEditorDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Template to edit, or `null` to create a new one (seeded from standard_meeting). */
  templateId: string | null;
  /** Source of the template being edited (drives the built-in→override subtext). */
  source: TemplateSource | null;
  /** Called after a successful save so the caller can refresh its list. */
  onSaved: () => void | Promise<void>;
}

/** An editable section row; a stable key keeps React state sane across reorders. */
interface EditableSection extends TemplateSection {
  key: number;
}

let nextSectionKey = 1;

function toEditable(sections: TemplateSection[]): EditableSection[] {
  return sections.map((section) => ({ ...section, key: nextSectionKey++ }));
}

function emptySection(): EditableSection {
  return { key: nextSectionKey++, title: '', instruction: '', format: 'paragraph' };
}

/**
 * Template editor (specs/0020 task 9): name, description, and ordered sections with
 * title / instruction / format (+ list-only item_format & example_item_format).
 * Saving posts the full template JSON (snake_case section fields) to
 * `api_save_template`; backend validation errors surface inline + as a toast.
 * Editing a built-in saves a same-id *override* — the built-in itself is untouched.
 */
export function TemplateEditorDialog({
  open,
  onOpenChange,
  templateId,
  source,
  onSaved,
}: TemplateEditorDialogProps) {
  const isNew = templateId === null;
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [sections, setSections] = useState<EditableSection[]>([]);

  // (Re)load the template whenever the dialog opens. A new template starts from a
  // copy of standard_meeting so the user edits a working example, not a blank form.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const load = async () => {
      setIsLoading(true);
      setLoadError(null);
      setSaveError(null);
      try {
        // NB: this command's Tauri arg is `template_id` → camelCase key `templateId`.
        const details = await invoke<TemplateDetails>('api_get_template_details', {
          templateId: templateId ?? NEW_TEMPLATE_SEED_ID,
        });
        if (cancelled) return;
        setName(isNew ? '' : details.name);
        setDescription(details.description ?? '');
        // Full per-section objects live in `section_details` (the legacy
        // `sections` field is titles-only and stays for older callers).
        setSections(toEditable(details.section_details ?? []));
      } catch (error) {
        console.error('Failed to load template details:', error);
        if (!cancelled) {
          setLoadError(typeof error === 'string' ? error : 'Could not load the template.');
        }
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [open, templateId, isNew]);

  const updateSection = (key: number, patch: Partial<TemplateSection>) => {
    setSections((prev) =>
      prev.map((section) => (section.key === key ? { ...section, ...patch } : section)),
    );
  };

  const moveSection = (index: number, delta: -1 | 1) => {
    setSections((prev) => {
      const target = index + delta;
      if (target < 0 || target >= prev.length) return prev;
      const next = [...prev];
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
  };

  const removeSection = (key: number) => {
    setSections((prev) => prev.filter((section) => section.key !== key));
  };

  const addSection = () => {
    setSections((prev) => [...prev, emptySection()]);
  };

  const handleSave = async () => {
    if (isSaving) return;
    setIsSaving(true);
    setSaveError(null);
    try {
      // Full template JSON with snake_case section fields (serde schema). Blank
      // list-only fields are omitted rather than sent as empty strings.
      const payload = {
        name: name.trim(),
        description: description.trim(),
        sections: sections.map(({ title, instruction, format, item_format, example_item_format }) => ({
          title: title.trim(),
          instruction: instruction.trim(),
          format,
          ...(format === 'list' && item_format?.trim()
            ? { item_format: item_format.trim() }
            : {}),
          ...(format === 'list' && example_item_format?.trim()
            ? { example_item_format: example_item_format.trim() }
            : {}),
        })),
      };
      const result = await invoke<{ id: string }>('api_save_template', {
        id: templateId,
        templateJson: JSON.stringify(payload),
      });
      toast.success(isNew ? 'Template created' : 'Template saved', {
        description: `"${payload.name}" will be available for summary generation.`,
      });
      console.log('Saved template:', result?.id);
      onOpenChange(false);
      await onSaved();
    } catch (error) {
      // Backend validation errors arrive as strings — surface them inline + toast.
      console.error('Failed to save template:', error);
      const message =
        typeof error === 'string'
          ? error
          : error instanceof Error
            ? error.message
            : 'Could not save the template.';
      setSaveError(message);
      toast.error('Could not save template', { description: message });
    } finally {
      setIsSaving(false);
    }
  };

  const editingBuiltIn = !isNew && source === 'builtin';

  return (
    <Dialog open={open} onOpenChange={(next) => (isSaving ? undefined : onOpenChange(next))}>
      <DialogContent className="flex max-h-[85vh] max-w-2xl flex-col">
        <DialogHeader>
          <DialogTitle>{isNew ? 'New template' : `Edit "${name || templateId}"`}</DialogTitle>
          <DialogDescription>
            {isNew
              ? 'Starts from a copy of the Standard Meeting template — rename it and shape the sections for your meeting type.'
              : editingBuiltIn
                ? 'This is a built-in template. Saving keeps the original and stores your changes as an edited version; delete the edited version later to revert.'
                : source === 'override'
                  ? 'You are editing your edited version of a built-in template.'
                  : 'Changes apply the next time a summary is generated with this template.'}
          </DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="flex items-center justify-center py-10 text-muted-foreground">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            Loading template…
          </div>
        ) : loadError ? (
          <div className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            {loadError}
          </div>
        ) : (
          <div className="-mr-2 flex-1 space-y-4 overflow-y-auto pr-2">
            {/* Name + description */}
            <div className="space-y-1.5">
              <Label htmlFor="template-name">Name</Label>
              <Input
                id="template-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. Weekly 1:1"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="template-description">Description</Label>
              <Textarea
                id="template-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="What kind of meeting is this template for?"
                rows={2}
              />
            </div>

            {/* Ordered sections */}
            <div className="flex items-center justify-between">
              <div className="text-sm font-medium">Sections</div>
              <Button variant="outline" size="sm" onClick={addSection}>
                <Plus className="h-4 w-4" />
                Add section
              </Button>
            </div>
            {sections.length === 0 && (
              <p className="text-sm text-muted-foreground">
                No sections yet — a template needs at least one.
              </p>
            )}
            {sections.map((section, index) => (
              <div key={section.key} className="space-y-3 rounded-lg border p-4">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Section {index + 1}
                  </span>
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() => moveSection(index, -1)}
                      disabled={index === 0}
                      aria-label={`Move section ${index + 1} up`}
                      title="Move up"
                    >
                      <ChevronUp className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() => moveSection(index, 1)}
                      disabled={index === sections.length - 1}
                      aria-label={`Move section ${index + 1} down`}
                      title="Move down"
                    >
                      <ChevronDown className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() => removeSection(section.key)}
                      aria-label={`Remove section ${index + 1}`}
                      title="Remove section"
                      className="text-destructive hover:text-destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_auto]">
                  <div className="space-y-1.5">
                    <Label htmlFor={`section-title-${section.key}`}>Title</Label>
                    <Input
                      id={`section-title-${section.key}`}
                      value={section.title}
                      onChange={(e) => updateSection(section.key, { title: e.target.value })}
                      placeholder="e.g. Action Items"
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor={`section-format-${section.key}`}>Format</Label>
                    <select
                      id={`section-format-${section.key}`}
                      value={section.format}
                      onChange={(e) =>
                        updateSection(section.key, { format: e.target.value as SectionFormat })
                      }
                      className="block h-9 rounded-md border border-input bg-background px-2 py-1 text-sm"
                    >
                      {SECTION_FORMATS.map((format) => (
                        <option key={format.value} value={format.value}>
                          {format.label}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor={`section-instruction-${section.key}`}>Instruction</Label>
                  <Textarea
                    id={`section-instruction-${section.key}`}
                    value={section.instruction}
                    onChange={(e) => updateSection(section.key, { instruction: e.target.value })}
                    placeholder="What should the AI write in this section?"
                    rows={3}
                  />
                </div>
                {section.format === 'list' && (
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    <div className="space-y-1.5">
                      <Label htmlFor={`section-item-format-${section.key}`}>
                        Item format <span className="text-muted-foreground">(optional)</span>
                      </Label>
                      <Input
                        id={`section-item-format-${section.key}`}
                        value={section.item_format ?? ''}
                        onChange={(e) => updateSection(section.key, { item_format: e.target.value })}
                        placeholder="e.g. [Owner] — [Task] by [Due date]"
                      />
                    </div>
                    <div className="space-y-1.5">
                      <Label htmlFor={`section-example-item-format-${section.key}`}>
                        Example item <span className="text-muted-foreground">(optional)</span>
                      </Label>
                      <Input
                        id={`section-example-item-format-${section.key}`}
                        value={section.example_item_format ?? ''}
                        onChange={(e) =>
                          updateSection(section.key, { example_item_format: e.target.value })
                        }
                        placeholder="e.g. Sam — send the deck by Friday"
                      />
                    </div>
                  </div>
                )}
              </div>
            ))}

            {/* Inline save/validation error (backend validator message). */}
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
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isSaving}>
            Cancel
          </Button>
          <Button
            variant="brand"
            onClick={handleSave}
            disabled={isSaving || isLoading || !!loadError || !name.trim()}
            title={!name.trim() ? 'Give the template a name first' : undefined}
          >
            {isSaving && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isNew ? 'Create template' : 'Save'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
