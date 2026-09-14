import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ChannelStrip } from '@/components/MeetingDetails/ChannelStrip';

describe('ChannelStrip', () => {
  it('renders one row per channel with time and share, CH1 first', () => {
    render(
      <ChannelStrip
        rows={[
          { channel: 1, speakerKey: 'local', colorClass: 'bg-chart-1', seconds: 1450, share: 0.58 },
          { channel: 2, speakerKey: 'spk_0', colorClass: 'bg-chart-2', seconds: 692, share: 0.27 },
        ]}
        renderName={(r) => <span>{r.speakerKey === 'local' ? 'You' : 'Sarah Chen'}</span>}
      />,
    );
    const rows = screen.getAllByRole('row');
    expect(rows.length).toBe(3); // header + 2
    expect(rows[1].textContent).toMatch(/1/);
    expect(rows[1].textContent).toMatch(/You/);
    expect(rows[1].textContent).toMatch(/24:10/);
    expect(rows[1].textContent).toMatch(/58%/);
    expect(rows[2].textContent).toMatch(/Sarah Chen/);
  });

  it('rounds percentages so the rows sum to 100 (largest remainder)', () => {
    render(
      <ChannelStrip
        rows={[
          { channel: 1, speakerKey: 'local', colorClass: 'bg-chart-1', seconds: 10, share: 0.125 },
          { channel: 2, speakerKey: 'spk_0', colorClass: 'bg-chart-2', seconds: 10, share: 0.125 },
          { channel: 3, speakerKey: 'spk_1', colorClass: 'bg-chart-3', seconds: 60, share: 0.75 },
        ]}
        renderName={(r) => <span>{r.speakerKey}</span>}
      />,
    );
    const pcts = screen
      .getAllByRole('row')
      .slice(1)
      .map((r) => {
        const cells = r.querySelectorAll('[role="cell"]');
        return Number((cells[cells.length - 1].textContent ?? '').replace('%', ''));
      });
    expect(pcts.reduce((a, b) => a + b, 0)).toBe(100);
    expect(pcts[2]).toBe(75);
  });
});
