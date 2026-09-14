/**
 * Resolve the summary language for a meeting (extracted from useSummaryGeneration so
 * both the meeting-details summary flow AND the deferred-backlog processor share one
 * implementation — low-power-mode spec §5). Precedence:
 *   1. an explicit per-meeting override (`readMeetingSummaryLanguage`);
 *   2. a previously cached auto-detection (`readCachedDetectedSummaryLanguage`);
 *   3. a fresh detection from the transcript (`detectAndCacheSummaryLanguage`).
 * Returns null to let the backend fall back to its own default.
 */

import { toast } from 'sonner';
import {
  detectAndCacheSummaryLanguage,
  readMeetingSummaryLanguage,
  readCachedDetectedSummaryLanguage,
} from '@/lib/summary-language-preferences';

export async function resolveSummaryLanguage(
  meetingId: string,
  transcriptTexts: string[],
): Promise<string | null> {
  try {
    const perMeeting = await readMeetingSummaryLanguage(meetingId);
    if (perMeeting.language) return perMeeting.language;
  } catch (err) {
    console.warn('Failed to load meeting summary language:', err);
    toast.warning('Could not load saved summary language', {
      description: 'Using Auto for this generation.',
    });
  }

  try {
    const cachedDetected = await readCachedDetectedSummaryLanguage(meetingId);
    if (cachedDetected) return cachedDetected;
  } catch (err) {
    console.warn('Failed to load cached detected summary language:', err);
  }

  try {
    const detection = await detectAndCacheSummaryLanguage(meetingId, transcriptTexts);
    if (detection.reason === 'tie') {
      toast.warning('Bilingual transcript detected', {
        description: 'Pick a summary language manually if Auto chooses the wrong fallback.',
      });
    }
    return detection.language;
  } catch (err) {
    console.warn('Failed to detect transcript summary language:', err);
    return null;
  }
}
