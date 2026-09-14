'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';

/**
 * "Your email" card (specs/0018) — the durable "which invite addresses are me"
 * set, so the user is never seeded as a participant in their own meetings and
 * their voice maps to "You". Stored only on this Mac.
 *
 * Extracted from RecordingSettings and moved to Settings → General (it governs
 * calendar seeding and identity, not recording). When Google Calendar connects,
 * the backend auto-adds the Google account email to this list (specs/0032) —
 * the window-focus re-fetch below picks that up after the user returns from the
 * browser consent flow.
 */
export function OwnerEmailSettings() {
  const [ownerEmails, setOwnerEmails] = useState<string[]>([]);
  const [ownerEmailInput, setOwnerEmailInput] = useState('');
  const [addingOwnerEmail, setAddingOwnerEmail] = useState(false);

  const loadOwnerEmails = useCallback(async () => {
    try {
      const emails = await invoke<string[]>('api_get_owner_emails');
      setOwnerEmails(Array.isArray(emails) ? emails : []);
    } catch (error) {
      console.error('Failed to load owner emails:', error);
    }
  }, []);

  // Load on mount, and re-fetch on window focus: the Google Calendar connect
  // flow round-trips through the browser and auto-adds the account email on
  // success, so refreshing when focus returns keeps this list current without
  // any new IPC.
  useEffect(() => {
    void loadOwnerEmails();
    const onFocus = () => void loadOwnerEmails();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [loadOwnerEmails]);

  // Add an owner email (specs/0018). Light email-shape validation; normalize to
  // lower+trim to match the backend store. Optimistic, revert on error, and adopt the
  // canonical list the command returns (the backend may dedupe/normalize differently).
  const handleAddOwnerEmail = async () => {
    const normalized = ownerEmailInput.trim().toLowerCase();
    if (!normalized) return;
    // Basic email shape: something@something.tld with no spaces.
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized)) {
      toast.error('Enter a valid email address');
      return;
    }
    if (ownerEmails.includes(normalized)) {
      toast.error('That address is already added');
      setOwnerEmailInput('');
      return;
    }
    const previous = ownerEmails;
    setAddingOwnerEmail(true);
    setOwnerEmails((prev) => [...prev, normalized]); // optimistic
    setOwnerEmailInput('');
    try {
      const updated = await invoke<string[]>('api_add_owner_email', { email: normalized });
      setOwnerEmails(Array.isArray(updated) ? updated : [...previous, normalized]);
    } catch (error) {
      console.error('Failed to add owner email:', error);
      setOwnerEmails(previous); // revert
      toast.error('Could not add address', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setAddingOwnerEmail(false);
    }
  };

  // Remove an owner email (specs/0018). Optimistic, revert on error. Note: this stops
  // future auto-matching but does not un-merge people already folded into "You".
  const handleRemoveOwnerEmail = async (email: string) => {
    const previous = ownerEmails;
    setOwnerEmails((prev) => prev.filter((e) => e !== email)); // optimistic
    try {
      const updated = await invoke<string[]>('api_remove_owner_email', { email });
      setOwnerEmails(Array.isArray(updated) ? updated : previous.filter((e) => e !== email));
    } catch (error) {
      console.error('Failed to remove owner email:', error);
      setOwnerEmails(previous); // revert
      toast.error('Could not remove address', {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <div className="border-t pt-6">
      <h4 className="text-base font-medium text-foreground mb-1">Your email</h4>
      <p className="text-sm text-muted-foreground mb-4">
        Tell Nixon which invite addresses are you, so you&apos;re never added as a
        participant and your own voice maps to &quot;You&quot;. Add any work, personal,
        or alias addresses you&apos;re invited under. Stored only on this Mac.
      </p>

      <div className="rounded-lg border p-4">
        {/* Existing addresses. */}
        {ownerEmails.length > 0 ? (
          <ul className="mb-3 space-y-2">
            {ownerEmails.map((email) => (
              <li
                key={email}
                className="flex items-center justify-between gap-2 rounded-md bg-muted px-3 py-1.5"
              >
                <span className="truncate text-sm text-foreground">{email}</span>
                <button
                  type="button"
                  onClick={() => void handleRemoveOwnerEmail(email)}
                  aria-label={`Remove ${email}`}
                  className="flex-shrink-0 rounded-[3px] p-1 text-muted-foreground hover:bg-background hover:text-foreground"
                >
                  <X size={14} />
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mb-3 text-sm text-muted-foreground">
            No addresses added yet.
          </p>
        )}

        {/* Add an address. */}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleAddOwnerEmail();
          }}
          className="flex items-center gap-2"
        >
          <input
            type="email"
            value={ownerEmailInput}
            onChange={(e) => setOwnerEmailInput(e.target.value)}
            placeholder="you@example.com"
            aria-label="Add your email address"
            className="flex-1 rounded-md border border-input px-2 py-1 text-sm"
          />
          <Button
            type="submit"
            size="sm"
            disabled={addingOwnerEmail || !ownerEmailInput.trim()}
          >
            Add
          </Button>
        </form>
        <p className="mt-2 text-xs text-muted-foreground">
          When Google Calendar is connected, your Google account email is added
          automatically.
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          Removing an address stops future auto-matching; it won&apos;t undo people
          already merged into you.
        </p>
      </div>
    </div>
  );
}
