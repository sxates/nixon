import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ChannelStrip } from '@/components/MeetingDetails/ChannelStrip';

/**
 * specs/0061 W4 (task 3) — clicking a speaker row filters the transcript and jumps
 * to their first line. This locks the ChannelStrip half: a row wired with
 * `onSelect` is keyboard- and mouse-activatable and reports its selected state.
 *
 * Controller ruling R4 amends the brief: `aria-selected` only has option semantics
 * (valid on `option`/`row`-in-a-grid/`tab`/etc.), and ChannelStrip's rows are plain
 * `role="row"` cells in a `role="table"` — not a listbox, and converting the whole
 * strip to listbox/option would tear out the existing columnheader/cell structure
 * two OTHER test files assert on (channel-strip-placement.test.tsx,
 * ChannelStrip.test.tsx). So a selectable row is exposed as `role="button"` with
 * `aria-pressed` (a real toggle button), only when the caller opts in via
 * `onSelect` — rows with no `onSelect` stay exactly `role="row"` as before.
 */
describe('ChannelStrip — selectable rows (specs/0061 W4)', () => {
  const rows = [
    { channel: 1, speakerKey: 'local', colorClass: 'bg-chart-1', seconds: 10, share: 0.4 },
    { channel: 2, speakerKey: 'spk_1', colorClass: 'bg-chart-2', seconds: 15, share: 0.6 },
  ];

  it('calls onSelect with the row key on click', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /spk_1/ }));
    expect(onSelect).toHaveBeenCalledWith('spk_1');
  });

  it('calls onSelect with the row key on Enter when the row itself is focused', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    const row = screen.getByRole('button', { name: /spk_1/ });
    row.focus();
    fireEvent.keyDown(row, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('spk_1');
  });

  it('reflects selectedKey via aria-pressed', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
        selectedKey="spk_1"
      />,
    );
    expect(screen.getByRole('button', { name: /local/ })).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByRole('button', { name: /spk_1/ })).toHaveAttribute('aria-pressed', 'true');
  });

  it('does not select when the click lands on a nested interactive control inside the row', () => {
    const onSelect = vi.fn();
    const onRename = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => (
          <button type="button" onClick={onRename}>
            {r.speakerKey}
          </button>
        )}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'spk_1' }));
    expect(onRename).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('leaves rows as plain role="row" when onSelect is not provided (unchanged behavior)', () => {
    render(<ChannelStrip rows={rows} renderName={(r) => <span>{r.speakerKey}</span>} />);
    expect(screen.getAllByRole('row')).toHaveLength(3); // header + 2
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
});
