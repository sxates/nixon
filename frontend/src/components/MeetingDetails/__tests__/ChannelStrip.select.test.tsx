import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ChannelStrip } from '@/components/MeetingDetails/ChannelStrip';

/**
 * specs/0061 W4 (task 3) — clicking a speaker row filters the transcript and jumps
 * to their first line. This locks the ChannelStrip half.
 *
 * Controller ruling R4 amended the brief's `aria-selected` (option-only semantics)
 * to a real toggle button. Controller ruling R37 then amended R4's first attempt
 * (the whole row as `role="button"`): that orphaned the row's `role="cell"`
 * children (a cell's required context role is `row`) and nested real `<button>`s
 * (rename, merge, dismiss) inside a widget role. The row now stays `role="row"`
 * with its cells intact, and the selection affordance is a genuine `<button>`
 * inside the channel cell — `aria-pressed`, named "Filter transcript to <name>" —
 * giving keyboard/VoiceOver users a real focusable path. The row keeps its own
 * click handler (mouse convenience) plus the nested-interactive guard so clicking
 * a nested control (rename, merge, or this very button) doesn't double-fire.
 */
describe('ChannelStrip — selectable rows (specs/0061 W4)', () => {
  const rows = [
    { channel: 1, speakerKey: 'local', colorClass: 'bg-chart-1', seconds: 10, share: 0.4, displayName: 'You' },
    { channel: 2, speakerKey: 'spk_1', colorClass: 'bg-chart-2', seconds: 15, share: 0.6, displayName: 'Maya Okafor' },
  ];

  it('calls onSelect with the row key when the row is clicked', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    // Click a plain (non-interactive) part of the row — the time cell.
    fireEvent.click(screen.getByText('0:15'));
    expect(onSelect).toHaveBeenCalledWith('spk_1');
  });

  it('calls onSelect with the row key when the channel cell button is activated', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Filter transcript to Maya Okafor' }));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith('spk_1');
  });

  it('the channel cell button is a real, natively-focusable <button> (keyboard/VoiceOver path)', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    const button = screen.getByRole('button', { name: 'Filter transcript to Maya Okafor' });
    expect(button.tagName).toBe('BUTTON');
    // Native <button> Enter/Space activation is the browser's job (jsdom doesn't
    // synthesize it), so this locks the CONTRACT a real button gives for free —
    // no custom keydown plumbing needed, unlike the row itself.
    button.focus();
    expect(document.activeElement).toBe(button);
  });

  it('reflects selectedKey via aria-pressed on the channel cell button, not the row', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
        selectedKey="spk_1"
      />,
    );
    expect(
      screen.getByRole('button', { name: 'Filter transcript to You' }),
    ).toHaveAttribute('aria-pressed', 'false');
    expect(
      screen.getByRole('button', { name: 'Filter transcript to Maya Okafor' }),
    ).toHaveAttribute('aria-pressed', 'true');
    // The row itself carries no aria-pressed/aria-selected — it's a plain `role="row"`.
    const rowEls = screen.getAllByRole('row');
    for (const row of rowEls) {
      expect(row).not.toHaveAttribute('aria-pressed');
      expect(row).not.toHaveAttribute('aria-selected');
    }
  });

  it('does not double-fire when a nested interactive control inside the row is clicked', () => {
    const onSelect = vi.fn();
    const onRename = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => (
          <button type="button" onClick={onRename}>
            rename {r.speakerKey}
          </button>
        )}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'rename spk_1' }));
    expect(onRename).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('does not double-fire when the channel cell button itself is clicked (button + row handlers)', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Filter transcript to Maya Okafor' }));
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('keeps the row and cells exactly as role="row"/role="cell" (table structure intact)', () => {
    const onSelect = vi.fn();
    render(
      <ChannelStrip
        rows={rows}
        renderName={(r) => <span>{r.speakerKey}</span>}
        onSelect={onSelect}
      />,
    );
    expect(screen.getAllByRole('row')).toHaveLength(3); // header + 2
    expect(screen.getAllByRole('cell').length).toBeGreaterThan(0);
  });

  it('leaves rows with no selection button when onSelect is not provided (unchanged behavior)', () => {
    render(<ChannelStrip rows={rows} renderName={(r) => <span>{r.speakerKey}</span>} />);
    expect(screen.getAllByRole('row')).toHaveLength(3); // header + 2
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
});
