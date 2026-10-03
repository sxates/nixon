import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn(), success: vi.fn(), info: vi.fn() }),
}));

import { PromptEditorDialog } from '../PromptEditorDialog';
import type { SummaryPrompt } from '@/lib/summary-prompts';

const existing: SummaryPrompt = {
  id: 'p1',
  name: 'Standup',
  body: 'Short bullets.',
  sort_order: 0,
  is_default: false,
  extract_action_items: false,
  in_library: true,
  created_at: '2026-01-01',
  updated_at: '2026-01-01',
};

function setup(prompt: SummaryPrompt | null = null) {
  const onOpenChange = vi.fn();
  const onSaved = vi.fn();
  render(
    <PromptEditorDialog open onOpenChange={onOpenChange} prompt={prompt} onSaved={onSaved} />,
  );
  return { onOpenChange, onSaved };
}

const saveButton = () => screen.getByRole('button', { name: /^(save|create prompt)$/i });

describe('PromptEditorDialog (specs/0079 W4)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('shows the help text and disables Save until name and body are filled', async () => {
    setup();
    expect(
      screen.getByText(
        "Tell the model how you want your notes shaped. Nixon always keeps its own rules: it only uses what was said, and it begins every summary with a title. Your prompt can't change those.",
      ),
    ).toBeInTheDocument();
    expect(saveButton()).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Name'), 'Weekly');
    expect(saveButton()).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Prompt'), '   ');
    expect(saveButton()).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Prompt'), 'Be brief');
    expect(saveButton()).toBeEnabled();
    expect(screen.getByLabelText('Name')).toHaveAttribute('maxlength', '60');
  });

  it('disables Save past 4000 chars and shows a destructive counter', async () => {
    setup();
    await userEvent.type(screen.getByLabelText('Name'), 'Big');
    const body = screen.getByLabelText('Prompt');
    await userEvent.click(body);
    await userEvent.paste('a'.repeat(4001));
    const counter = screen.getByText('4001 / 4000');
    expect(counter).toHaveClass('text-destructive');
    expect(saveButton()).toBeDisabled();
  });

  it('creates with id null and extractActionItems defaulting to true', async () => {
    invokeMock.mockResolvedValue({ ...existing, id: 'new' });
    const { onOpenChange, onSaved } = setup();
    expect(screen.getByRole('switch', { name: 'Extract action items' })).toBeChecked();
    await userEvent.type(screen.getByLabelText('Name'), ' Weekly ');
    await userEvent.type(screen.getByLabelText('Prompt'), 'Be brief');
    await userEvent.click(saveButton());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_save_summary_prompt', {
        id: null,
        name: 'Weekly',
        body: 'Be brief',
        extractActionItems: true,
      }),
    );
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(onSaved).toHaveBeenCalled();
  });

  it('edits with the id and honours the toggle', async () => {
    invokeMock.mockResolvedValue(existing);
    setup(existing);
    const toggle = screen.getByRole('switch', { name: 'Extract action items' });
    expect(toggle).not.toBeChecked();
    await userEvent.click(toggle);
    await userEvent.click(saveButton());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_save_summary_prompt', {
        id: 'p1',
        name: 'Standup',
        body: 'Short bullets.',
        extractActionItems: true,
      }),
    );
  });

  it('shows a rejected save inline and keeps the dialog open', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    invokeMock.mockRejectedValue('A prompt named "Standup" already exists.');
    const { onOpenChange, onSaved } = setup(existing);
    await userEvent.click(saveButton());
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'A prompt named "Standup" already exists.',
    );
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });
});
