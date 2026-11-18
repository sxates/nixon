'use client';

import { useCallback, useEffect, useState } from 'react';
import { DEFAULT_RECORD_VIEW, readRecordView, writeRecordView, type RecordView } from '@/lib/record-view';

/**
 * The record screen's Transcript / Focus choice, remembered across recordings. Starts at the
 * default and reads the stored choice after mount, so the server render and the first client
 * render agree.
 */
export function useRecordView(): [RecordView, (view: RecordView) => void] {
  const [view, setViewState] = useState<RecordView>(DEFAULT_RECORD_VIEW);
  useEffect(() => setViewState(readRecordView()), []);
  const setView = useCallback((next: RecordView) => {
    setViewState(next);
    writeRecordView(next);
  }, []);
  return [view, setView];
}
