import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { OneOffPromptDialog } from '../OneOffPromptDialog';
import { PROMPT_NOTES_HINT } from '../PromptNotesHint';

function setup(onSubmit: (body: string, extract: boolean) => Promise<void> = vi.fn(async () => {})) {
  const onOpenChange = vi.fn();
  render(
    <OneOffPromptDialog
      open
      onOpenChange={onOpenChange}
      initialBody=""
      initialExtract
      onSubmit={onSubmit}
    />,
  );
  return { onSubmit, onOpenChange };
}

const submit = () => screen.getByRole('button', { name: 'Use for this meeting' });

describe('OneOffPromptDialog (specs/0079 W4)', () => {
  it('tells the user they can refer to "my notes" and ties the hint to the textarea', () => {
    setup();
    expect(screen.getByText(PROMPT_NOTES_HINT)).toBeInTheDocument();
    expect(PROMPT_NOTES_HINT).toContain('"my notes"');
    expect(screen.getByLabelText('Prompt')).toHaveAccessibleDescription(PROMPT_NOTES_HINT);
  });

  it('disables submit when empty', () => {
    setup();
    expect(submit()).toBeDisabled();
  });

  it('disables submit over 4000 characters and flags the counter', () => {
    setup();
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'x'.repeat(4001) } });
    expect(submit()).toBeDisabled();
    expect(screen.getByText('4001 / 4000')).toHaveClass('text-destructive');
  });

  it('defaults Extract action items to checked and submits (body, extract)', async () => {
    const onSubmit = vi.fn(async () => {});
    setup(onSubmit);
    const box = screen.getByLabelText('Extract action items');
    expect(box).toBeChecked();
    await userEvent.type(screen.getByLabelText('Prompt'), 'Be brief');
    await userEvent.click(box);
    await userEvent.click(submit());
    expect(onSubmit).toHaveBeenCalledWith('Be brief', false);
  });

  it('stays open when submit rejects', async () => {
    const { onOpenChange } = setup(
      vi.fn(async () => {
        throw new Error('nope');
      }),
    );
    await userEvent.type(screen.getByLabelText('Prompt'), 'Be brief');
    await userEvent.click(submit());
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Prompt')).toHaveValue('Be brief');
    expect(submit()).toBeEnabled();
  });

  it('Cancel closes', async () => {
    const { onOpenChange } = setup();
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
