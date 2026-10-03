import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { SaveOneOffDialog } from '../SaveOneOffDialog';

type SaveOpts = { name?: string; toLibrary: boolean; toSeries: boolean };

function setup(hasSeries: boolean, onSave: (opts: SaveOpts) => Promise<void> = vi.fn(async () => {})) {
  const onSkip = vi.fn();
  render(
    <SaveOneOffDialog
      open
      onOpenChange={vi.fn()}
      hasSeries={hasSeries}
      defaultName="Focus on decisions"
      onSave={onSave}
      onSkip={onSkip}
    />,
  );
  return { onSave, onSkip };
}

const save = () => screen.getByRole('button', { name: 'Save' });

describe('SaveOneOffDialog (specs/0079 W4)', () => {
  it('omits the series checkbox when the meeting has no series', () => {
    setup(false);
    expect(screen.queryByLabelText('Use for future meetings in this series')).toBeNull();
    expect(screen.getByLabelText('Save to my prompt library')).toBeInTheDocument();
  });

  it('disables Save until something is checked', async () => {
    setup(true);
    expect(save()).toBeDisabled();
    await userEvent.click(screen.getByLabelText('Use for future meetings in this series'));
    expect(save()).toBeEnabled();
  });

  it('requires a name when saving to the library (prefilled, max 60)', async () => {
    setup(false);
    await userEvent.click(screen.getByLabelText('Save to my prompt library'));
    const name = screen.getByLabelText('Prompt name');
    expect(name).toHaveValue('Focus on decisions');
    expect(name).toHaveAttribute('maxlength', '60');
    await userEvent.clear(name);
    expect(save()).toBeDisabled();
    await userEvent.type(name, 'Mine');
    expect(save()).toBeEnabled();
  });

  it('passes { name, toLibrary, toSeries } to onSave', async () => {
    const { onSave } = setup(true);
    await userEvent.click(screen.getByLabelText('Use for future meetings in this series'));
    await userEvent.click(screen.getByLabelText('Save to my prompt library'));
    await userEvent.click(save());
    expect(onSave).toHaveBeenCalledWith({
      name: 'Focus on decisions',
      toLibrary: true,
      toSeries: true,
    });
  });

  it('Not now calls onSkip', async () => {
    const { onSkip } = setup(true);
    await userEvent.click(screen.getByRole('button', { name: 'Not now' }));
    expect(onSkip).toHaveBeenCalled();
  });

  it('stays open and re-enables Save when onSave rejects', async () => {
    const { onSkip } = setup(
      false,
      vi.fn(async () => {
        throw new Error('nope');
      }),
    );
    await userEvent.click(screen.getByLabelText('Save to my prompt library'));
    await userEvent.click(save());
    expect(onSkip).not.toHaveBeenCalled();
    expect(save()).toBeEnabled();
  });
});
