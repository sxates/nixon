import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { SegmentSpeakerMenu } from '@/components/MeetingDetails/SegmentSpeakerMenu';

// specs/0019 WS2.3 (note 8) — move ONE transcript line to another speaker. Locks:
// (a) no control when there's nothing to move to, (b) picking a different speaker
// reassigns by transcript id, (c) picking the current speaker is a no-op.

const SPEAKERS = [
  { speakerKey: 'local', displayName: 'You' },
  { speakerKey: 'spk_0', displayName: 'Speaker 1' },
  { speakerKey: 'spk_1', displayName: 'Speaker 2' },
];

describe('SegmentSpeakerMenu (WS2.3)', () => {
  it('renders nothing when there are fewer than two speakers', () => {
    const { container } = render(
      <SegmentSpeakerMenu
        transcriptId="t1"
        currentSpeakerKey="spk_0"
        speakers={[{ speakerKey: 'spk_0', displayName: 'Speaker 1' }]}
        onReassign={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('reassigns the line to the chosen speaker by transcript id', async () => {
    const onReassign = vi.fn().mockResolvedValue(undefined);
    render(
      <SegmentSpeakerMenu
        transcriptId="t1"
        currentSpeakerKey="spk_0"
        speakers={SPEAKERS}
        onReassign={onReassign}
      />,
    );

    const trigger = screen.getByRole('button', { name: /move this line/i });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Speaker 2'));

    await waitFor(() => expect(onReassign).toHaveBeenCalledWith('t1', 'spk_1'));
  });

  it('does not reassign when the current speaker is chosen', async () => {
    const onReassign = vi.fn().mockResolvedValue(undefined);
    render(
      <SegmentSpeakerMenu
        transcriptId="t1"
        currentSpeakerKey="spk_0"
        speakers={SPEAKERS}
        onReassign={onReassign}
      />,
    );

    const trigger = screen.getByRole('button', { name: /move this line/i });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Speaker 1')); // the current one

    // Give any (incorrect) async call a tick to land.
    await new Promise((r) => setTimeout(r, 0));
    expect(onReassign).not.toHaveBeenCalled();
  });
});
