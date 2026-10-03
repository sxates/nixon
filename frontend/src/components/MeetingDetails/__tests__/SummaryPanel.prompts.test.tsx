import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn(), info: vi.fn() }) }));
vi.mock('@/hooks/useDiarizationActive', () => ({ useDiarizationActive: () => false }));
vi.mock('@/components/AISummary/BlockNoteSummaryView', async () => {
  const { forwardRef } = await import('react');
  return { BlockNoteSummaryView: forwardRef(() => <div data-testid="summary-view" />) };
});
vi.mock('@/components/EmptyStateSummary', () => ({ EmptyStateSummary: () => null }));
vi.mock('../SummaryGenerating', () => ({ SummaryGenerating: () => null }));

import { SummaryPanel } from '../SummaryPanel';

const prompts = [
  { id: 'p-default', name: 'Default notes' },
  { id: 'p-standup', name: 'Standup' },
];
const state = (over: object = {}) => ({
  source: 'default',
  prompt_id: 'p-default',
  prompt_name: 'Default notes',
  extract_action_items: true,
  custom_body: null,
  custom_extract_action_items: null,
  has_series: true,
  ...over,
});

function setup(opts: { hasSummary?: boolean } = {}) {
  const hasSummary = opts.hasSummary ?? true;
  const onRegenerateSummary = vi.fn(async () => {});
  // A stateful fake of the hook bundle so the panel sees the refreshed state.
  const api = {
    prompts,
    state: state(),
    selectPrompt: vi.fn(async () => {}),
    saveOneOff: vi.fn(async () => {}),
    clearOneOff: vi.fn(async () => {}),
    saveFollowup: vi.fn(async () => ({})),
  };
  const props = {
    meeting: { id: 'm1', title: 'T', created_at: '2026-01-01' },
    meetingTitle: 'T',
    onTitleChange: vi.fn(),
    isEditingTitle: false,
    onStartEditTitle: vi.fn(),
    onFinishEditTitle: vi.fn(),
    isTitleDirty: false,
    summaryRef: { current: null },
    isSaving: false,
    onSaveAll: vi.fn(async () => {}),
    onCopySummary: vi.fn(async () => {}),
    aiSummary: hasSummary ? ({ markdown: '# hi' } as never) : null,
    summaryStatus: 'completed' as const,
    transcripts: [{ id: 't1' }] as never,
    modelConfig: { provider: 'ollama', model: 'gemma2:2b' } as never,
    setModelConfig: vi.fn(),
    onSaveModelConfig: vi.fn(async () => {}),
    onGenerateSummary: vi.fn(async () => {}),
    onStopGeneration: vi.fn(),
    onSaveSummary: vi.fn(async () => {}),
    onSummaryChange: vi.fn(),
    onDirtyChange: vi.fn(),
    summaryError: null,
    onRegenerateSummary,
  };
  const ui = (a: typeof api) => <SummaryPanel {...props} promptsApi={a as never} />;
  const utils = render(ui(api));
  return { api, onRegenerateSummary, rerenderWith: (a: typeof api) => utils.rerender(ui(a)) };
}

async function openCustomDialog() {
  await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
  await userEvent.click(screen.getByRole('menuitem', { name: /custom for this meeting/i }));
}

describe('SummaryPanel prompt flows (specs/0079 W4)', () => {
  beforeEach(() => vi.clearAllMocks());

  it('asks for confirmation before persisting a different prompt, then regenerates once the pick persisted', async () => {
    const { api, onRegenerateSummary, rerenderWith } = setup();
    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Standup' }));

    expect(await screen.findByText('Regenerate summary?')).toBeInTheDocument();
    expect(api.selectPrompt).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'Regenerate' }));
    await waitFor(() => expect(api.selectPrompt).toHaveBeenCalledWith('p-standup'));
    // Not yet: the refreshed state hasn't reported the new prompt.
    expect(onRegenerateSummary).not.toHaveBeenCalled();

    rerenderWith({ ...api, state: state({ source: 'meeting', prompt_id: 'p-standup', prompt_name: 'Standup' }) });
    await waitFor(() => expect(onRegenerateSummary).toHaveBeenCalledTimes(1));
  });

  it('Cancel leaves the selection untouched', async () => {
    const { api, onRegenerateSummary } = setup();
    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Standup' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(api.selectPrompt).not.toHaveBeenCalled();
    expect(onRegenerateSummary).not.toHaveBeenCalled();
  });

  it('persists silently when there is no summary yet', async () => {
    const { api } = setup({ hasSummary: false });
    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Standup' }));
    expect(api.selectPrompt).toHaveBeenCalledWith('p-standup');
    expect(screen.queryByText('Regenerate summary?')).toBeNull();
  });

  it('one-off submit opens the follow-up dialog; Save then regenerates exactly once', async () => {
    const { api, onRegenerateSummary } = setup();
    await openCustomDialog();
    await userEvent.type(await screen.findByLabelText('Prompt'), 'Be brief');
    await userEvent.click(screen.getByRole('button', { name: 'Use for this meeting' }));

    expect(await screen.findByLabelText('Use for future meetings in this series')).toBeInTheDocument();
    expect(api.saveOneOff).toHaveBeenCalledWith('Be brief', true);
    expect(onRegenerateSummary).not.toHaveBeenCalled();

    await userEvent.click(screen.getByLabelText('Use for future meetings in this series'));
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(api.saveFollowup).toHaveBeenCalledWith({
        name: undefined,
        toLibrary: false,
        toSeries: true,
      }),
    );
    await waitFor(() => expect(onRegenerateSummary).toHaveBeenCalledTimes(1));
  });

  it('Not now skips the follow-up but still regenerates', async () => {
    const { api, onRegenerateSummary } = setup();
    await openCustomDialog();
    await userEvent.type(await screen.findByLabelText('Prompt'), 'Be brief');
    await userEvent.click(screen.getByRole('button', { name: 'Use for this meeting' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(onRegenerateSummary).toHaveBeenCalledTimes(1));
    expect(api.saveFollowup).not.toHaveBeenCalled();
  });

  it('does not regenerate when no summary exists yet', async () => {
    const { onRegenerateSummary } = setup({ hasSummary: false });
    await openCustomDialog();
    await userEvent.type(await screen.findByLabelText('Prompt'), 'Be brief');
    await userEvent.click(screen.getByRole('button', { name: 'Use for this meeting' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Not now' })).toBeNull());
    expect(onRegenerateSummary).not.toHaveBeenCalled();
  });

  it('a rejected one-off save keeps the dialog open and never regenerates', async () => {
    const { api, onRegenerateSummary } = setup();
    api.saveOneOff.mockRejectedValueOnce(new Error('boom'));
    await openCustomDialog();
    await userEvent.type(await screen.findByLabelText('Prompt'), 'Be brief');
    await userEvent.click(screen.getByRole('button', { name: 'Use for this meeting' }));
    await waitFor(() => expect(api.saveOneOff).toHaveBeenCalled());
    expect(screen.getByLabelText('Prompt')).toHaveValue('Be brief');
    expect(onRegenerateSummary).not.toHaveBeenCalled();
  });
});
