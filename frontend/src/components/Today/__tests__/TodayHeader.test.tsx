import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

// specs/0069 W3 — Today's header lost its `New note` and `Record` buttons in favor of
// one primary `Add meeting` (Record duplicated the always-on transport rail's REC key
// plus ⌘⇧R; New note went unused). This is the only test that would catch either of
// those regressing back in, since `AddMeetingDialog.test.tsx` only exercises the dialog
// in isolation and never renders it through the header.

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/components/CommandPalette/SearchMeetingsButton', () => ({
  SearchMeetingsButton: vi.fn(() => null),
}));
vi.mock('@/components/DeferredBacklog/ProcessMeetingsButton', () => ({
  ProcessMeetingsButton: vi.fn(() => null),
}));

import { TodayHeader } from '@/components/Today/TodayHeader';

describe('TodayHeader (specs/0069 W3)', () => {
  it('renders one Add meeting action and calls onAddMeeting when clicked', () => {
    const onAddMeeting = vi.fn();
    render(<TodayHeader now={new Date('2026-09-20T10:00:00')} daySummary="Sunday" onAddMeeting={onAddMeeting} />);

    const addButton = screen.getByRole('button', { name: /add meeting/i });
    addButton.click();
    expect(onAddMeeting).toHaveBeenCalledTimes(1);
  });

  it('no longer offers Record or New note', () => {
    render(<TodayHeader now={new Date('2026-09-20T10:00:00')} daySummary="Sunday" onAddMeeting={vi.fn()} />);

    expect(screen.queryByRole('button', { name: /^record$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /recording…/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /new note/i })).not.toBeInTheDocument();
  });
});
