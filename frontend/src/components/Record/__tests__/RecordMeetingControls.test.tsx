import React from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

// Owner feedback v0.18 (round 4): Participants and the summary prompt sit at the far right
// of the record screen's top strip, drawn like Pause transcript (xs outline keys on bg-card).

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));
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
    render(<RecordMeetingControls meetingId="m1" isRecordingActive promptsApi={api()} />);
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
    render(<RecordMeetingControls meetingId="m1" isRecordingActive promptsApi={api({ selectPrompt })} />);
    await userEvent.click(screen.getByRole('button', { name: 'Summary prompt: Standup' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Retro' }));
    expect(selectPrompt).toHaveBeenCalledWith('p2');

    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: /custom for this Meeting/i }));
    expect(await screen.findByRole('dialog', { name: /custom prompt for this Meeting/i })).toBeInTheDocument();
  });

  it('renders nothing before a recording or a meeting row exists', () => {
    const { container } = render(<RecordMeetingControls meetingId={null} isRecordingActive={false} promptsApi={api()} />);
    expect(container).toBeEmptyDOMElement();
  });
});
