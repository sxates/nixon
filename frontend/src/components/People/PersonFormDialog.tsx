'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
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
import { Textarea } from '@/components/ui/textarea';
import type { Person } from '@/types';

interface PersonFormDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** When set, the dialog edits this person; otherwise it creates a new one. */
  person?: Person | null;
  /** Called after a successful create/update (refetch the list). */
  onSaved: () => void | Promise<void>;
}

/** Trim to a non-empty string, or `undefined` for the optional IPC args. */
function optional(value: string): string | undefined {
  const trimmed = value.trim();
  return trimmed === '' ? undefined : trimmed;
}

/**
 * Create-or-edit form for a person (specs/0016 1b). Create uses
 * `api_create_person` (upsert-by-email); edit uses `api_update_person`. The
 * voiceprint opt-out toggle lives on the directory row/detail, not here.
 */
export function PersonFormDialog({ open, onOpenChange, person, onSaved }: PersonFormDialogProps) {
  const isEdit = !!person;
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [role, setRole] = useState('');
  const [notes, setNotes] = useState('');
  const [isSaving, setIsSaving] = useState(false);

  // Seed fields whenever the dialog opens (or the target person changes).
  useEffect(() => {
    if (!open) return;
    setDisplayName(person?.displayName ?? '');
    setEmail(person?.email ?? '');
    setRole(person?.role ?? '');
    setNotes(person?.notes ?? '');
  }, [open, person]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (isSaving) return;
    const name = displayName.trim();
    if (!name) {
      toast.error('A name is required');
      return;
    }
    setIsSaving(true);
    try {
      if (isEdit && person) {
        await invoke('api_update_person', {
          id: person.id,
          displayName: name,
          email: optional(email) ?? null,
          role: optional(role) ?? null,
          notes: optional(notes) ?? null,
        });
        toast.success(`Saved ${name}`);
      } else {
        await invoke('api_create_person', {
          displayName: name,
          email: optional(email) ?? null,
          role: optional(role) ?? null,
          notes: optional(notes) ?? null,
        });
        toast.success(`Added ${name}`);
      }
      onOpenChange(false);
      await onSaved();
    } catch (error) {
      console.error('Failed to save person:', error);
      toast.error(isEdit ? 'Could not save this person.' : 'Could not add this person.', {
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
            <DialogTitle>{isEdit ? 'Edit person' : 'Add a person'}</DialogTitle>
            <DialogDescription>
              People are remembered across meetings, so you can attach a name to a detected speaker
              once and reuse it.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-3 py-4">
            <div className="space-y-1.5">
              <Label htmlFor="person-name">Name</Label>
              <Input
                id="person-name"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder="Priya Sharma"
                autoFocus
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="person-email">
                Email <span className="font-normal text-muted-foreground">(optional)</span>
              </Label>
              <Input
                id="person-email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="priya@example.com"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="person-role">
                Role <span className="font-normal text-muted-foreground">(optional)</span>
              </Label>
              <Input
                id="person-role"
                value={role}
                onChange={(e) => setRole(e.target.value)}
                placeholder="Product Manager"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="person-notes">
                Notes <span className="font-normal text-muted-foreground">(optional)</span>
              </Label>
              <Textarea
                id="person-notes"
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder="Anything worth remembering about this person"
                className="min-h-[72px] resize-none"
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
              {isEdit ? 'Save' : 'Add person'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
