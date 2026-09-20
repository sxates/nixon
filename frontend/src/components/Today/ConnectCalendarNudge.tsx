'use client';

import { useCallback, useState } from 'react';
import { toast } from 'sonner';
import { ConnectCalendarCard } from '@/components/Calendar/ConnectCalendarCard';
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
    <ConnectCalendarCard
      title="Connect your calendar to see your full day"
      connecting={connecting}
      onConnect={() => void handleConnect()}
    />
  );
}
