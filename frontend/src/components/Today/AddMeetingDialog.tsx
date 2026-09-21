'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
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
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { localDateKey } from '@/lib/today-timeline';
import { createManualMeeting, updateManualMeeting, type ManualMeetingInput } from '@/lib/day-agenda';

const DURATION_OPTIONS_MIN = [15, 30, 45, 60, 90] as const;
const DEFAULT_DURATION_MIN = 30;

interface EditingMeeting {
  meetingId: string;
  title: string;
  startsAt: string;
  endsAt: string | null;
  joinUrl: string | null;
}

interface AddMeetingDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The day the timeline is showing, `YYYY-MM-DD` — the form's default date. */
  defaultDateKey: string;
  /** Editing an existing entry; omitted when adding. */
  editing?: EditingMeeting;
  /** Called after a successful create/update so the caller can refresh the agenda. */
  onSaved: () => void | Promise<void>;
}

/** `HH:MM` from a `Date`'s LOCAL hour/minute (never UTC). */
function toLocalTimeStr(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** The next half-hour mark from `now`, as an `HH:MM` local time string. */
function nextHalfHourStr(now: Date = new Date()): string {
  const next = new Date(now);
  next.setSeconds(0, 0);
  const minutes = next.getMinutes();
  next.setMinutes(minutes < 30 ? 30 : 0, 0, 0);
  if (minutes >= 30) next.setHours(next.getHours() + 1);
  return toLocalTimeStr(next);
}

/**
 * A local calendar-day + wall-clock instant built from separate `date` (`YYYY-MM-DD`)
 * and `time` (`HH:MM`) fields, as an absolute ISO instant. `new Date(\`${date}T${time}\`)`
 * (no offset) parses as LOCAL time — exactly what the user typed on their own clock —
 * and `.toISOString()` then carries that instant to the backend. Never assemble the UTC
 * string by hand.
 */
function localInstant(date: string, time: string): Date {
  return new Date(`${date}T${time}`);
}

/**
 * Add/edit a manually added meeting (specs/0069 W3) — a meeting on today's timeline
 * with no calendar event behind it. Mirrors `PersonFormDialog`'s create-or-edit shape:
 * `editing` present -> `api_update_manual_meeting`, absent -> `api_create_manual_meeting`.
 */
export function AddMeetingDialog({
  open,
  onOpenChange,
  defaultDateKey,
  editing,
  onSaved,
}: AddMeetingDialogProps) {
  const isEdit = !!editing;
  const [title, setTitle] = useState('');
  const [dateStr, setDateStr] = useState(defaultDateKey);
  const [timeStr, setTimeStr] = useState(() => nextHalfHourStr());
  const [durationMin, setDurationMin] = useState<number>(DEFAULT_DURATION_MIN);
  const [joinUrl, setJoinUrl] = useState('');
  const [isSaving, setIsSaving] = useState(false);

  // Seed fields whenever the dialog opens (or the target entry changes) — same pattern
  // as PersonFormDialog.
  useEffect(() => {
    if (!open) return;
    if (editing) {
      const start = new Date(editing.startsAt);
      setTitle(editing.title);
      setDateStr(localDateKey(start));
      setTimeStr(toLocalTimeStr(start));
      if (editing.endsAt) {
        const end = new Date(editing.endsAt);
        const minutes = Math.round((end.getTime() - start.getTime()) / 60000);
        setDurationMin(minutes > 0 ? minutes : DEFAULT_DURATION_MIN);
      } else {
        setDurationMin(DEFAULT_DURATION_MIN);
      }
      setJoinUrl(editing.joinUrl ?? '');
    } else {
      setTitle('');
      setDateStr(defaultDateKey);
      setTimeStr(nextHalfHourStr());
      setDurationMin(DEFAULT_DURATION_MIN);
      setJoinUrl('');
    }
  }, [open, editing, defaultDateKey]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (isSaving) return;
    const trimmedTitle = title.trim();
    if (!trimmedTitle) {
      toast.error('A title is required');
      return;
    }
    const start = localInstant(dateStr, timeStr);
    if (Number.isNaN(start.getTime())) {
      toast.error('Enter a valid date and start time');
      return;
    }
    const end = new Date(start.getTime() + durationMin * 60000);
    const input: ManualMeetingInput = {
      title: trimmedTitle,
      startsAt: start.toISOString(),
      endsAt: end.toISOString(),
      joinUrl: joinUrl.trim() || null,
    };

    setIsSaving(true);
    try {
      if (isEdit && editing) {
        await updateManualMeeting(editing.meetingId, input);
      } else {
        await createManualMeeting(input);
      }
      toast.success(isEdit ? 'Meeting updated' : 'Meeting added');
      onOpenChange(false);
      await onSaved();
    } catch (error) {
      console.error('Failed to save manual meeting:', error);
      toast.error(isEdit ? 'Could not save the meeting' : 'Could not add the meeting', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isSaving ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{isEdit ? 'Edit meeting' : 'Add meeting'}</DialogTitle>
            <DialogDescription>
              Put a call on your day. It shows up on the timeline like any other meeting, ready
              to join and record.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-3 py-4">
            <div className="space-y-1.5">
              <Label htmlFor="meeting-title">Title</Label>
              <Input
                id="meeting-title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Call with Sam"
                autoFocus
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <Label htmlFor="meeting-date">Date</Label>
                <Input
                  id="meeting-date"
                  type="date"
                  value={dateStr}
                  onChange={(e) => setDateStr(e.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="meeting-start">Start</Label>
                <Input
                  id="meeting-start"
                  type="time"
                  value={timeStr}
                  onChange={(e) => setTimeStr(e.target.value)}
                />
              </div>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="meeting-duration">Duration</Label>
              <Select
                value={String(durationMin)}
                onValueChange={(v) => setDurationMin(Number(v))}
              >
                <SelectTrigger id="meeting-duration">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {DURATION_OPTIONS_MIN.map((minutes) => (
                    <SelectItem key={minutes} value={String(minutes)}>
                      {minutes} minutes
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="meeting-join-url">
                Join link <span className="font-normal text-muted-foreground">(optional)</span>
              </Label>
              <Input
                id="meeting-join-url"
                type="url"
                value={joinUrl}
                onChange={(e) => setJoinUrl(e.target.value)}
                placeholder="https://zoom.us/j/…"
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSaving}
            >
              Cancel
            </Button>
            <Button type="submit" variant="brand" disabled={isSaving}>
              {isSaving && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {isEdit ? 'Save' : 'Add meeting'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
