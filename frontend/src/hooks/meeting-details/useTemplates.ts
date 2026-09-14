import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

/**
 * Fallback template when a meeting has no persisted choice (`template_id` NULL).
 *
 * specs/0053 W3: Auto derives the section list from the meeting's own content
 * instead of imposing one of six author-time layouts. Meetings with an
 * explicitly persisted template_id keep it — this is the default for new
 * meetings only.
 */
export const DEFAULT_TEMPLATE_ID = 'auto';

/**
 * The Auto entry. Not a backend template — `api_list_templates` never returns
 * it (the id is reserved server-side), so the picker contributes it locally and
 * pins it first.
 */
export const AUTO_TEMPLATE = {
  id: 'auto',
  name: 'Auto (recommended)',
  description: "Structure chosen from the meeting's content",
  hidden: false,
} as const;

/**
 * True when `id` looks like a real SQLite `meetings` row id that per-meeting state
 * may be persisted against. Guards against the two fabricated ids that float around
 * the frontend and must never receive writes (specs/0024 WS3.1, specs/0029 WS4.3):
 * - the sidebar's `'intro-call'` placeholder, and
 * - TranscriptContext's IndexedDB id, `meeting-<Date.now()>` (an all-digit suffix —
 *   real rows are `meeting-<uuid>`, which always contains hex letters/hyphens).
 */
export function isPersistableMeetingId(id: string | null | undefined): id is string {
  if (!id || !id.trim()) return false;
  if (id === 'intro-call') return false;
  if (/^meeting-\d+$/.test(id)) return false;
  return true;
}

/**
 * True when `title` is worth asking the backend for a template suggestion
 * (specs/0020 task 10). Empty titles and the auto-generated date-stamp titles
 * ("Meeting " followed by a digit) carry no recurring-meeting signal, so the
 * suggestion call is skipped for them.
 */
export function isSuggestableTitle(title: string | null | undefined): title is string {
  const trimmed = title?.trim();
  if (!trimmed) return false;
  if (/^Meeting \d/.test(trimmed)) return false;
  return true;
}

/**
 * Pure decision for the confirm-before-regenerate flow (specs/0020 task 8):
 * picking a *different* template on a meeting that already shows a summary must
 * ask before anything persists — regeneration costs minutes on local models.
 * With no summary (or the same template), the selection persists silently.
 */
export function shouldConfirmTemplateChange(
  nextTemplateId: string,
  currentTemplateId: string,
  hasExistingSummary: boolean,
): boolean {
  return hasExistingSummary && nextTemplateId !== currentTemplateId;
}

/**
 * Summary-template list + selection, persisted per meeting (specs/0029 WS4.3 — the
 * specs/0020 `meetings.template_id` slice).
 *
 * @param meetingId Persistence target. Pass it explicitly where an authoritative id
 *   exists (the record screen passes `activeRecordingMeetingId`, which is briefly
 *   `null` before the row is created — selections made in that window are queued and
 *   flushed once the id arrives). When omitted entirely (meeting-details), the hook
 *   falls back to the sidebar's viewed meeting (`currentMeeting`), which that page
 *   sets to the meeting being displayed.
 * @param meetingTitle The meeting's current title, used only for the
 *   auto-select-by-title suggestion (specs/0020 task 10): when the meeting has no
 *   persisted template and the user hasn't picked one, the most recent prior
 *   meeting with the same title donates its template as the default.
 */
export function useTemplates(meetingId?: string | null, meetingTitle?: string | null) {
  const [allTemplates, setAllTemplates] = useState<Array<{
    id: string;
    name: string;
    description: string;
    hidden?: boolean;
  }>>([]);
  const [selectedTemplate, setSelectedTemplate] = useState<string>(DEFAULT_TEMPLATE_ID);

  // Pickers only show templates the user hasn't removed — EXCEPT the meeting's
  // current selection, which must stay visible (its label/checkmark) even if the
  // user hid that template after this meeting adopted it. Auto is pinned first —
  // it's not a backend template (the id is reserved server-side and never
  // returned by api_list_templates), so it's contributed locally here.
  const availableTemplates = [
    AUTO_TEMPLATE,
    ...allTemplates.filter((t) => !t.hidden || t.id === selectedTemplate),
  ];

  // NOTE: an explicit argument — even null — wins over the fallback. On the record
  // screen the viewed-meeting global can point at a *different* meeting the user
  // browsed mid-recording, so falling back there would retarget the write (the same
  // trap as specs/0029 WS5.1).
  const hasExplicitId = meetingId !== undefined;
  const { currentMeeting } = useSidebar();
  const candidateId = hasExplicitId ? meetingId : currentMeeting?.id;
  const targetMeetingId = isPersistableMeetingId(candidateId) ? candidateId : null;

  // A selection made before the meeting row exists is queued here and flushed as
  // soon as the id shows up.
  const pendingSaveRef = useRef<string | null>(null);
  // Once the user actively picks a template this mount, a late-resolving persisted
  // load must not clobber their choice.
  const userSelectedRef = useRef(false);

  // Fetch available templates on mount
  useEffect(() => {
    const fetchTemplates = async () => {
      try {
        const templates = await invokeTauri('api_list_templates') as Array<{
          id: string;
          name: string;
          description: string;
          hidden?: boolean;
        }>;
        console.log('Available templates:', templates);
        setAllTemplates(templates);
      } catch (error) {
        console.error('Failed to fetch templates:', error);
      }
    };
    fetchTemplates();
  }, []);

  const persistSelection = useCallback(async (id: string, templateId: string) => {
    try {
      await invokeTauri('api_set_meeting_template', { meetingId: id, templateId });
    } catch (error) {
      console.error('Failed to save template choice:', error);
      toast.error('Could not save template choice', {
        description: 'The template will be used for now, but may not persist.',
      });
    }
  }, []);

  // Armed when the persisted load came back empty for this meeting id: the
  // title-based suggestion below may then propose a default (specs/0020 task 10).
  const [pendingSuggestFor, setPendingSuggestFor] = useState<string | null>(null);

  // Load the persisted per-meeting template. NULL/absent keeps the default and
  // arms the title-based suggestion.
  useEffect(() => {
    if (!targetMeetingId) return;
    let cancelled = false;
    setPendingSuggestFor(null);
    const loadPersisted = async () => {
      try {
        const persisted = await invokeTauri('api_get_meeting_template', {
          meetingId: targetMeetingId,
        }) as string | null;
        if (cancelled || userSelectedRef.current) return;
        if (typeof persisted === 'string' && persisted) {
          setSelectedTemplate(persisted);
        } else {
          // No persisted choice — a same-title prior meeting may donate one.
          setPendingSuggestFor(targetMeetingId);
        }
      } catch (error) {
        // Non-fatal: generation just proceeds with the default template.
        console.warn('Failed to load persisted template:', error);
      }
    };
    loadPersisted();
    return () => {
      cancelled = true;
    };
  }, [targetMeetingId]);

  // Auto-select-by-title (specs/0020 task 10): only when nothing is persisted
  // (armed above) and the user hasn't picked. A suggestion becomes the displayed
  // selection AND is persisted immediately, so every generation path (manual,
  // Day-Agenda one-click, auto-summarize) sees the same default. Waits silently
  // for a usable title — the record screen's title can arrive after the row id.
  useEffect(() => {
    if (!pendingSuggestFor || pendingSuggestFor !== targetMeetingId) return;
    if (userSelectedRef.current) return;
    if (!isSuggestableTitle(meetingTitle)) return; // keep armed; the title may still arrive
    const title = meetingTitle.trim();
    let cancelled = false;
    const suggest = async () => {
      try {
        const suggested = await invokeTauri('api_suggest_template_for_title', {
          title,
          excludeMeetingId: pendingSuggestFor,
        }) as string | null;
        if (cancelled) return;
        setPendingSuggestFor(null); // one shot per meeting once a real title was tried
        if (userSelectedRef.current) return;
        if (typeof suggested === 'string' && suggested) {
          setSelectedTemplate(suggested);
          void persistSelection(pendingSuggestFor, suggested);
        }
      } catch (error) {
        // Non-fatal: the default template stands.
        console.warn('Template suggestion by title failed:', error);
        if (!cancelled) setPendingSuggestFor(null);
      }
    };
    void suggest();
    return () => {
      cancelled = true;
    };
  }, [pendingSuggestFor, targetMeetingId, meetingTitle, persistSelection]);

  // Flush a queued selection once the meeting row id arrives.
  useEffect(() => {
    if (targetMeetingId && pendingSaveRef.current) {
      const queued = pendingSaveRef.current;
      pendingSaveRef.current = null;
      void persistSelection(targetMeetingId, queued);
    }
  }, [targetMeetingId, persistSelection]);

  // Handle template selection
  const handleTemplateSelection = useCallback((templateId: string, templateName: string) => {
    userSelectedRef.current = true;
    setSelectedTemplate(templateId);
    if (targetMeetingId) {
      void persistSelection(targetMeetingId, templateId);
    } else {
      pendingSaveRef.current = templateId;
    }
    toast.success('Template selected', {
      description: `Using "${templateName}" template for summary generation`,
    });
  }, [targetMeetingId, persistSelection]);

  return {
    availableTemplates,
    selectedTemplate,
    handleTemplateSelection,
  };
}
