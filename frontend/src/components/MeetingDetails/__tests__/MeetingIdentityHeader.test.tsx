import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { MeetingIdentityHeader } from '@/components/MeetingDetails/MeetingIdentityHeader';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const base = {
  meetingId: 'm-1',
  title: 'Quarterly planning',
  onTitleChange: vi.fn(),
  onSaveTitle: vi.fn(),
  // 10:00 local, so the identity line's time segment is deterministic wherever CI runs.
  createdAt: new Date(2026, 5, 24, 10, 0, 0).toISOString(),
};

describe('MeetingIdentityHeader', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([{ id: 'm-1', durationSeconds: 2538 }]);
  });

  it('engraves the reel handle, date, time and tape length onto the identity line', async () => {
    render(<MeetingIdentityHeader {...base} reelNumber={412} source="Zoom" />);

    const line = screen.getByTestId('meeting-identity-line');
    expect(line.textContent).toContain('REEL 0412');
    expect(line.textContent).toContain('Jun 24');
    expect(line.textContent).toContain('10:00');
    expect(line.textContent).toContain('Zoom');
    // Duration arrives from api_get_meetings, as a tape counter.
    await waitFor(() => expect(line.textContent).toContain('00:42:18'));
    expect(line.className).toContain('u-section-label');
  });

  it('prints the blank reel handle and drops unknown segments', async () => {
    invokeMock.mockResolvedValue([]);
    render(<MeetingIdentityHeader {...base} reelNumber={null} />);

    const line = await screen.findByTestId('meeting-identity-line');
    // No duration and no source: exactly three segments, no dangling separator.
    const segments = line.textContent!.split(' · ');
    expect(segments).toHaveLength(3);
    expect(segments[0]).toBe('REEL ——');
    expect(segments[2]).toBe('10:00');
  });

  it('renders the title at the 22px display size with no reel label card', async () => {
    render(<MeetingIdentityHeader {...base} reelNumber={412} source="Zoom" voices={4} />);

    const heading = await screen.findByRole('heading', { name: 'Quarterly planning' });
    expect(heading.className).toContain('font-display');
    expect(heading.className).toContain('text-[22px]');
    // 0.1.0 canvas feedback: the typed card is gone; the identity line carries everything,
    // including the voice count that only the card used to show.
    expect(screen.queryByTestId('reel-label-header')).not.toBeInTheDocument();
    expect(screen.getByTestId('meeting-identity-line').textContent).toContain('REEL 0412');
    expect(screen.getByTestId('meeting-identity-line').textContent).toContain('4 voices');
  });

  // The back button used to hang in the gutter beside the centred reading column
  // (`lg:absolute lg:-left-9`). specs/0064 W4 moved this header across the full width, where
  // that 36px pull dragged it into the page's own padding and hard against the sidebar
  // (owner feedback 2026-09-19). It is inline again, inside that padding.
  it('keeps the back button inline, not pulled out of the page padding', () => {
    render(<MeetingIdentityHeader {...base} onBack={() => {}} />);
    const back = screen.getByRole('button', { name: 'Back to meetings' });
    expect(back.className).not.toContain('lg:absolute');
    expect(back.className).not.toContain('lg:-left-9');
    // -ml-1 is optical alignment of the glyph, not a pull out of the container.
    expect(back.className).toContain('-ml-1');
  });
});
