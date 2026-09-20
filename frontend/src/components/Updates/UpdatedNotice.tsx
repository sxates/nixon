'use client';

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';

interface UpdateReceipt {
  version: string;
  notes: string;
}

/**
 * The first launch after an in-app update (specs/0069 W6).
 *
 * The restart used to be silent at both ends: nothing warned you the app would close (W5),
 * and nothing acknowledged that it had changed when it came back. The backend leaves a
 * receipt just before restarting and hands it over exactly once, so this shows the release
 * notes the update dialog would have shown, then never again for that version.
 *
 * A manual .dmg install leaves no receipt and gets no dialog — dragging an app into
 * /Applications is not a moment that needs explaining.
 */
export function UpdatedNotice() {
  const [receipt, setReceipt] = useState<UpdateReceipt | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<UpdateReceipt | null>('api_take_update_receipt')
      .then((r) => {
        if (cancelled || !r) return;
        setReceipt(r);
        setOpen(true);
      })
      // Not in Tauri, or the command is unavailable: there is nothing to announce.
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  if (!receipt) return null;
  const notes = receipt.notes.trim();

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Nixon updated to v{receipt.version}</DialogTitle>
        </DialogHeader>
        {notes.length > 0 ? (
          <div className="max-h-[50vh] overflow-y-auto text-sm [&_h3]:text-[12px] [&_li]:text-sm [&_p]:text-sm">
            <AnswerMarkdown markdown={notes} />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Everything is where you left it — your meetings, notes and settings came along.
          </p>
        )}
        <div className="flex justify-end">
          <Button variant="brand" onClick={() => setOpen(false)}>
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
