import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SegmentTextEditor } from '../SegmentTextEditor';

// specs/0061 W5 (task 5) — the inline transcript-line editor. Key handling per the
// task brief: Enter saves (trimmed), Shift+Enter inserts a newline (the textarea's
// own default — nothing to assert beyond "it doesn't save"), Escape cancels without
// saving, and blur saves. Escape and blur must not fight: cancelling must not also
// fire a save for the same focus-loss.

describe('SegmentTextEditor (specs/0061 W5)', () => {
  it('opens with the given initial text', () => {
    render(<SegmentTextEditor initialText="the anodised part" onSave={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole('textbox')).toHaveValue('the anodised part');
  });

  it('Enter saves the trimmed text', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={vi.fn()} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.change(textarea, { target: { value: '  corrected text  ' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });

    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledWith('corrected text');
  });

  it('Shift+Enter does not save (inserts a newline instead)', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={vi.fn()} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true });

    expect(onSave).not.toHaveBeenCalled();
  });

  it('Escape cancels without saving', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    const onCancel = vi.fn();
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={onCancel} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.change(textarea, { target: { value: 'a discarded edit' } });
    fireEvent.keyDown(textarea, { key: 'Escape' });

    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onSave).not.toHaveBeenCalled();

    // A blur following the Escape (the field losing focus as it's torn down) must
    // not ALSO fire a save of the discarded text.
    fireEvent.blur(textarea);
    expect(onSave).not.toHaveBeenCalled();
  });

  it('blur saves the trimmed text', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={vi.fn()} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.change(textarea, { target: { value: '  blurred save  ' } });
    fireEvent.blur(textarea);

    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledWith('blurred save');
  });

  it('a blur after Enter-save does not fire a second save', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={vi.fn()} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.keyDown(textarea, { key: 'Enter' });
    expect(onSave).toHaveBeenCalledTimes(1);

    fireEvent.blur(textarea);
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it('uses the transcript text class (u-typed)', () => {
    render(<SegmentTextEditor initialText="x" onSave={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole('textbox')).toHaveClass('u-typed');
  });
});
