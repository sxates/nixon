import { invoke as invokeTauri } from '@tauri-apps/api/core';
import type { Transcript } from '@/types';

interface TranscriptPage {
  transcripts: Transcript[];
  total_count: number;
  has_more: boolean;
}

/**
 * Every transcript row for a meeting — NOT the paginated view buffer.
 *
 * Copy, summary generation and the channel strip's share-of-talk all need the whole
 * meeting: the panel only holds `DEFAULT_PAGE_SIZE` (100) rows, so anything derived from
 * that buffer silently reports first-page numbers on a long meeting. Probes the count
 * with a `limit: 1` call, then asks for all of them. Throws on failure — callers decide
 * how loudly to complain.
 */
export async function fetchAllTranscripts(meetingId: string): Promise<Transcript[]> {
  const firstPage = (await invokeTauri('api_get_meeting_transcripts', {
    meetingId,
    limit: 1,
    offset: 0,
  })) as TranscriptPage;

  const totalCount = firstPage.total_count;
  if (totalCount === 0) return [];

  const allData = (await invokeTauri('api_get_meeting_transcripts', {
    meetingId,
    limit: totalCount,
    offset: 0,
  })) as TranscriptPage;
  return allData.transcripts;
}
