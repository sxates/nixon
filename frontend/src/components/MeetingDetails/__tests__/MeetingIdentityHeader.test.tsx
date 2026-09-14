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

  it('renders the title at the 22px display size and shows the reel label card', async () => {
    render(<MeetingIdentityHeader {...base} reelNumber={412} source="Zoom" />);

    const heading = await screen.findByRole('heading', { name: 'Quarterly planning' });
    expect(heading.className).toContain('font-display');
    expect(heading.className).toContain('text-[22px]');
    // The card is aria-hidden (decorative duplicate of the identity line above),
    // so it's found by test id rather than accessible name.
    expect(screen.getByTestId('reel-label-header')).toBeInTheDocument();
    expect(screen.getByTestId('reel-label-header').textContent).toContain('REEL 0412');
  });
});
