/**
 * One summary poll per meeting, shared by every caller that is waiting on it.
 *
 * The meeting page (`useSummaryGeneration`) and the deferred backlog both wait on a meeting's
 * summary through this. It used to keep ONE callback per meeting, and a second start replaced
 * the first — so when the backlog began summarizing a meeting the page was already summarizing,
 * the page's callback was dropped and it said "writing your summary" forever over a summary
 * that had finished (measured 2026-09-23). Now a start adds a subscriber; every subscriber
 * hears every update, and the poll ends for all of them at a terminal status.
 */

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type SummaryPollResult = any;
export type SummaryPollCallback = (result: SummaryPollResult) => void;

const TERMINAL = ['completed', 'error', 'failed', 'cancelled'];

export interface SummaryPoller {
  start: (meetingId: string, onUpdate: SummaryPollCallback) => void;
  /** Ends the meeting's poll for every subscriber (a cancel, or the backlog giving up). */
  stop: (meetingId: string) => void;
  stopAll: () => void;
}

export function createSummaryPoller(
  fetchSummary: (meetingId: string) => Promise<SummaryPollResult>,
  intervalMs = 5000,
  // ~16.5 minutes at 5s — slightly longer than the backend's 15-minute timeout.
  maxPolls = 200,
): SummaryPoller {
  const polls = new Map<
    string,
    { interval: ReturnType<typeof setInterval>; subscribers: Set<SummaryPollCallback>; resetTimeout: () => void }
  >();

  const end = (meetingId: string) => {
    const poll = polls.get(meetingId);
    if (!poll) return;
    clearInterval(poll.interval);
    polls.delete(meetingId);
  };

  const start = (meetingId: string, onUpdate: SummaryPollCallback) => {
    const existing = polls.get(meetingId);
    if (existing) {
      existing.subscribers.add(onUpdate);
      // A new run joining the poll gets the full timeout, as a restart always did.
      existing.resetTimeout();
      return;
    }

    const subscribers = new Set<SummaryPollCallback>([onUpdate]);
    const notify = (result: SummaryPollResult) => subscribers.forEach((cb) => cb(result));
    let pollCount = 0;

    const interval = setInterval(async () => {
      pollCount++;
      if (pollCount >= maxPolls) {
        console.warn(`⏱️ Polling timeout for ${meetingId} after ${maxPolls} iterations`);
        end(meetingId);
        notify({
          status: 'error',
          error: 'Summary generation timed out after 15 minutes. Please try again or check your model configuration.',
        });
        return;
      }
      try {
        const result = await fetchSummary(meetingId);
        // A stop() while the fetch was in flight means nobody is listening any more.
        if (polls.get(meetingId)?.interval !== interval) return;
        if (TERMINAL.includes(result.status) || (result.status === 'idle' && pollCount > 1)) {
          end(meetingId);
        }
        notify(result);
      } catch (error) {
        if (polls.get(meetingId)?.interval !== interval) return;
        console.error(`Polling error for ${meetingId}:`, error);
        end(meetingId);
        notify({ status: 'error', error: error instanceof Error ? error.message : 'Unknown error' });
      }
    }, intervalMs);

    polls.set(meetingId, { interval, subscribers, resetTimeout: () => { pollCount = 0; } });
  };

  return {
    start,
    stop: end,
    stopAll: () => [...polls.keys()].forEach(end),
  };
}
