import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { DashboardMeeting } from '@/components/Meetings/MeetingRow';

/**
 * Every recorded meeting, newest first (`api_get_meetings`) — the Meetings List view's
 * data, and the count in its header. `enabled: false` skips the read until it is needed.
 */
export function useRecordedMeetings(enabled: boolean) {
  const [meetings, setMeetings] = useState<DashboardMeeting[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const load = async () => {
      try {
        const result = (await invoke('api_get_meetings', { authToken: null })) as DashboardMeeting[];
        if (!cancelled) {
          setMeetings(Array.isArray(result) ? result : []);
          setError(null);
        }
      } catch (err) {
        console.error('Failed to load meetings:', err);
        if (!cancelled) setError('Could not load your meetings.');
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  const removeMeeting = useCallback((id: string) => {
    setMeetings((prev) => prev.filter((m) => m.id !== id));
  }, []);

  return { meetings, isLoading, error, removeMeeting };
}
