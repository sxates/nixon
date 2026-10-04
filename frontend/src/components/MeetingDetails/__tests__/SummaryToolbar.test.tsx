import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * specs/0064 W3 — the owner's report was "buttons below the Summary tab feel overkill".
 * The row carried up to eight controls (Stop/Generate, AI Model, Prompt, a language
 * picker, Save, Copy). It is now three: the primary action, the prompt picker, and a "…"
 * flyout — with Save jumping out of the flyout the moment there is something unsaved,
 * because that is the one control that is ever urgent.
 */

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn(), info: vi.fn() }) }));

import { SummaryToolbar } from '@/components/MeetingDetails/SummaryToolbar';

const defaultState = {
  source: 'default',
  prompt_id: 'p-default',
  prompt_name: 'Default notes',
  extract_action_items: true,
  custom_body: null,
  custom_extract_action_items: null,
  has_series: false,
};

const base = {
  summaryStatus: 'completed' as const,
  hasSummary: true,
  hasTranscripts: true,
  isModelConfigLoading: false,
  onGenerateSummary: vi.fn(async () => {}),
  onStopGeneration: vi.fn(),
  prompts: [
    { id: 'p-default', name: 'Default notes' },
    { id: 'p-standup', name: 'Standup' },
  ] as never,
  promptState: defaultState as never,
  onPromptSelect: vi.fn(),
  onCustomPrompt: vi.fn(),
  onClearCustomPrompt: vi.fn(),
  isSaving: false,
  isDirty: false,
  onSave: vi.fn(async () => {}),
  onCopy: vi.fn(async () => {}),
  modelConfig: { provider: 'ollama', model: 'gemma2:2b' } as never,
  setModelConfig: vi.fn(),
  onSaveModelConfig: vi.fn(async () => {}),
};

describe('SummaryToolbar (specs/0064 W3, specs/0079)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it('shows three controls at rest', () => {
    render(<SummaryToolbar {...base} />);

    expect(screen.getByRole('button', { name: /regenerate summary/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /summary prompt/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /more summary actions/i })).toBeInTheDocument();
    // Everything else has moved into the flyout.
    expect(screen.queryByRole('button', { name: /^save$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^copy$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /ai model/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /re-think structure/i })).not.toBeInTheDocument();
  });

  it('promotes Save out of the flyout the moment there are unsaved edits', () => {
    render(<SummaryToolbar {...base} isDirty />);
    expect(screen.getByRole('button', { name: /save/i })).toBeInTheDocument();
  });

  it('labels the picker with the active prompt and lists prompts plus Custom', async () => {
    render(<SummaryToolbar {...base} />);
    await userEvent.click(screen.getByRole('button', { name: 'Summary prompt: Default notes' }));
    expect(screen.getByRole('menuitem', { name: /Default notes/ })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Standup' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: /custom for this meeting/i })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: /remove custom prompt/i })).not.toBeInTheDocument();
  });

  it('selecting a prompt reports its id and name', async () => {
    const onPromptSelect = vi.fn();
    render(<SummaryToolbar {...base} onPromptSelect={onPromptSelect} />);
    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Standup' }));
    expect(onPromptSelect).toHaveBeenCalledWith('p-standup', 'Standup');
  });

  it('opens the custom flow and offers removal only for a custom prompt', async () => {
    const onCustomPrompt = vi.fn();
    const onClearCustomPrompt = vi.fn();
    render(
      <SummaryToolbar
        {...base}
        onCustomPrompt={onCustomPrompt}
        onClearCustomPrompt={onClearCustomPrompt}
        promptState={{ ...defaultState, source: 'custom', prompt_id: null } as never}
      />,
    );
    await userEvent.click(
      screen.getByRole('button', { name: 'Summary prompt: Custom (this meeting)' }),
    );
    await userEvent.click(screen.getByRole('menuitem', { name: /custom for this meeting/i }));
    expect(onCustomPrompt).toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: /summary prompt/i }));
    await userEvent.click(screen.getByRole('menuitem', { name: /remove custom prompt/i }));
    expect(onClearCustomPrompt).toHaveBeenCalled();
  });

  it('has no Re-think structure anywhere', async () => {
    render(<SummaryToolbar {...base} />);
    await userEvent.click(screen.getByRole('button', { name: /more summary actions/i }));
    expect(screen.queryByRole('menuitem', { name: /re-think structure/i })).not.toBeInTheDocument();
  });

  it('offers Stop instead of Regenerate while a summary is generating', () => {
    render(<SummaryToolbar {...base} summaryStatus="summarizing" />);

    expect(screen.getByRole('button', { name: /stop/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /regenerate summary/i })).not.toBeInTheDocument();
  });

  it('says Generate, not Regenerate, before the first summary', () => {
    render(<SummaryToolbar {...base} hasSummary={false} />);
    expect(screen.getByRole('button', { name: /generate summary/i })).toBeInTheDocument();
  });

  it('renders nothing without transcripts to summarize', () => {
    const { container } = render(<SummaryToolbar {...base} hasTranscripts={false} />);
    expect(container).toBeEmptyDOMElement();
  });
});
