'use client';

import { useCallback, useState } from 'react';
import { Calendar } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { requestCalendarAccess } from '@/lib/calendar';

/** Compact "connect your calendar" nudge (mirrors the Day Agenda affordance). */
export function ConnectCalendarNudge({ onConnected }: { onConnected: () => void }) {
  const [connecting, setConnecting] = useState(false);
  const handleConnect = useCallback(async () => {
    setConnecting(true);
    try {
      const granted = await requestCalendarAccess();
      if (granted) onConnected();
      else
        toast.error('Calendar access not granted', {
          description: 'Grant access in System Settings → Privacy & Security → Calendars.',
        });
    } catch (err) {
      toast.error('Could not connect calendar', {
        description: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setConnecting(false);
    }
  }, [onConnected]);

  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
      <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-muted">
        <Calendar className="h-4 w-4 text-muted-foreground" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-foreground">
          Connect your calendar to see your full day
        </div>
        <div className="text-xs text-muted-foreground">
          Nixon reads your macOS Calendar on-device — nothing leaves your machine.
        </div>
      </div>
      <Button variant="brand" size="sm" className="flex-shrink-0" onClick={handleConnect} disabled={connecting}>
        {connecting ? 'Connecting…' : 'Connect'}
      </Button>
    </div>
  );
}
