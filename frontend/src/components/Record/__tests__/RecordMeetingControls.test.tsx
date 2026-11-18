import React from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

// Owner feedback v0.18: the meeting bar heading the record screen's left / main column —
// Topic chips + "Add Topic" on the left (no label), Participants + summary prompt on the
// right, every key drawn like Pause transcript (xs outline keys on bg-card); an empty band
// before the meeting row exists.

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) =>
    cmd === 'api_topics_for_meeting' ? [{ id: 't1', name: 'Launch', status: 'open' }] : [],
  ),
}));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));

import { RecordMeetingControls } from '@/components/Record/RecordMeetingControls';
import type { PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';

const api = (over: Partial<Record<string, unknown>> = {}) =>
  ({
    prompts: [
      { id: 'p1', name: 'Standup' },
      { id: 'p2', name: 'Retro' },
    ],
    state: { source: 'meeting', prompt_id: 'p1', prompt_name: 'Standup', has_series: false },
    selectPrompt: vi.fn(),
    saveOneOff: vi.fn(),
    clearOneOff: vi.fn(),
    saveFollowup: vi.fn(),
    ...over,
  }) as unknown as PromptsApi;

describe('RecordMeetingControls', () => {
  it('Participants then the prompt, both in the Pause transcript key style', () => {
    render(<RecordMeetingControls meetingId="m1" meetingTitle="Weekly sync" isRecordingActive promptsApi={api()} />);
    const participants = screen.getByRole('button', { name: 'Participants' });
    const prompt = screen.getByRole('button', { name: 'Summary prompt: Standup' });
    expect(participants.compareDocumentPosition(prompt) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    for (const key of [participants, prompt]) {
      expect(key.className).toMatch(/\bborder\b/);
      expect(key.className).toMatch(/\bh-7\b/);
      expect(key.className).toMatch(/\bbg-card\b/);
    }
    expect(prompt.className).toContain('max-w-[160px]');
  });

  it('selects a saved prompt and opens the one-off dialog from its new place', async () => {
    const selectPrompt = vi.fn();
    render(<RecordMeetingControls meetingId="m1" meetingTitle="Weekly sync" isRecordingActive promptsApi={api({ selectPrompt })} />);
    await userEvent.click(screen.getByRole('button', { name: 'Summary prompt: Standup' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Retro' }));
    expect(selectPrompt).toHaveBeenCalledWith('p2');

    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: /custom for this Meeting/i }));
    expect(await screen.findByRole('dialog', { name: /custom prompt for this Meeting/i })).toBeInTheDocument();
  });

  it('Topics on the left: chips then "Add Topic", no label, all in the strip key style', async () => {
    render(<RecordMeetingControls meetingId="m1" meetingTitle="Weekly sync" isRecordingActive promptsApi={api()} />);
    const chip = await screen.findByRole('button', { name: 'Launch' });
    const add = screen.getByRole('button', { name: 'Add Topic' });
    const participants = screen.getByRole('button', { name: 'Participants' });
    const following = (a: Node, b: Node) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
    expect(following(chip, add) && following(add, participants)).toBe(true);
    expect(screen.queryByText('Topics')).toBeNull();
    expect(screen.queryByText(/Add to Topic/)).toBeNull();
    for (const key of [chip, add]) {
      expect(key.className).toMatch(/\bh-7\b/);
      expect(key.className).toMatch(/\bbg-card\b/);
    }
    const bar = screen.getByTestId('record-meeting-bar');
    expect(bar.className).toContain('min-h-11');
    expect(bar.className).toContain('border-b');
  });

  it('before the meeting row exists: an empty band, so the rules still line up', () => {
    render(<RecordMeetingControls meetingId={null} meetingTitle={null} isRecordingActive={false} promptsApi={api()} />);
    const bar = screen.getByTestId('record-meeting-bar');
    expect(bar.className).toContain('min-h-11');
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });
});
