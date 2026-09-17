import { describe, it, expect } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useMeetingTabs } from '@/hooks/meeting-details/useMeetingTabs';

// specs/0060 — the screenshot pipeline deep-links `/meeting-details?tab=<key>` straight
// to a tab. page-content.tsx seeds this from the URL as `requestedTab`; it must win over
// the default (but the legacy `wantsPrepTab`/scheduled/notes-only/deep-link rules stay
// unchanged for callers that don't pass it).

const base = { wantsPrepTab: false, isScheduled: false, isNotesOnly: false, deepLinkSegmentId: null };

describe('useMeetingTabs requestedTab (specs/0060)', () => {
  it('seeds the active tab from ?tab= when valid', () => {
    const { result } = renderHook(() => useMeetingTabs({ ...base, requestedTab: 'transcript' } as any));
    expect(result.current.activeTab).toBe('transcript');
  });

  it('ignores an unknown value and keeps the default', () => {
    const { result } = renderHook(() => useMeetingTabs({ ...base, requestedTab: 'bogus' as any } as any));
    expect(result.current.activeTab).toBe('summary');
  });

  it('prep via requestedTab equals the legacy wantsPrepTab', () => {
    const { result } = renderHook(() => useMeetingTabs({ ...base, requestedTab: 'prep' } as any));
    expect(result.current.activeTab).toBe('prep');
  });

  it('leaves existing wantsPrepTab behaviour unchanged when requestedTab is absent', () => {
    const { result } = renderHook(() => useMeetingTabs({ ...base, wantsPrepTab: true } as any));
    expect(result.current.activeTab).toBe('prep');
  });

  it('a segment deep link wins over ?tab=: ends on Transcript even when requestedTab is summary', () => {
    const { result } = renderHook(() =>
      useMeetingTabs({ ...base, requestedTab: 'summary', deepLinkSegmentId: 'seg-1' } as any),
    );
    expect(result.current.activeTab).toBe('transcript');
  });
});
