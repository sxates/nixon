import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

/**
 * specs/0064 W3 — the owner's report was "buttons below the Summary tab feel overkill".
 * The row carried up to eight controls (Stop/Generate, AI Model, Template, Re-think
 * structure, a language picker, Save, Copy). It is now three: the primary action, the
 * template picker, and a "…" flyout — with Save jumping out of the flyout the moment
 * there is something unsaved, because that is the one control that is ever urgent.
 */

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn(), info: vi.fn() }) }));

import { SummaryToolbar } from '@/components/MeetingDetails/SummaryToolbar';

const base = {
  summaryStatus: 'completed' as const,
  hasSummary: true,
  hasTranscripts: true,
  isModelConfigLoading: false,
  onGenerateSummary: vi.fn(async () => {}),
  onStopGeneration: vi.fn(),
  availableTemplates: [
    { id: 'auto', name: 'Auto', description: 'Derived from the transcript' },
    { id: 'standup', name: 'Standup', description: 'Standup notes' },
  ],
  selectedTemplate: 'auto',
  onTemplateSelect: vi.fn(),
  isSaving: false,
  isDirty: false,
  onSave: vi.fn(async () => {}),
  onCopy: vi.fn(async () => {}),
  modelConfig: { provider: 'ollama', model: 'gemma2:2b' } as never,
  setModelConfig: vi.fn(),
  onSaveModelConfig: vi.fn(async () => {}),
  meetingId: 'm1',
  onRegenerate: vi.fn(async () => {}),
};

describe('SummaryToolbar (specs/0064 W3)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it('shows three controls at rest', () => {
    render(<SummaryToolbar {...base} />);

    expect(screen.getByRole('button', { name: /regenerate summary/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /summary template/i })).toBeInTheDocument();
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



  it('hides Re-think structure unless the template is Auto and a summary exists', async () => {
    render(<SummaryToolbar {...base} selectedTemplate="standup" />);
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
