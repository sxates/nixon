import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SegmentTextEditor } from '../SegmentTextEditor';

// specs/0061 W5 (task 5) — the inline transcript-line editor. Key handling per the
// task brief: Enter saves (trimmed), Shift+Enter does not save (the textarea's own
// default newline-insertion applies — jsdom can't assert that native behavior, only
// that no save fired), Escape cancels without saving, and blur saves. Escape and
// blur must not fight: cancelling must not also fire a save for the same focus-loss.
//
// specs/0061 review, I2 — a no-op edit (trimmed text unchanged from initialText) is
// treated as a cancel, not a save: it must invoke onCancel, never onSave.

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

  it('Shift+Enter does not save', () => {
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
    fireEvent.change(textarea, { target: { value: 'changed text' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    expect(onSave).toHaveBeenCalledTimes(1);

    fireEvent.blur(textarea);
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  // specs/0061 review, I2 — clicking the pencil and clicking away (or hitting Enter)
  // without changing anything is the natural "changed my mind" gesture and must not
  // mark the line edited or discard its word timestamps server-side.
  it('a no-op blur (text unchanged) cancels instead of saving', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    const onCancel = vi.fn();
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={onCancel} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.blur(textarea);

    expect(onSave).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('a no-op Enter (text unchanged after trimming whitespace) cancels instead of saving', () => {
    const onSave = vi.fn().mockResolvedValue(true);
    const onCancel = vi.fn();
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={onCancel} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.change(textarea, { target: { value: '  original  ' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });

    expect(onSave).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('a failed save keeps the typed text and allows a retry (specs/0061 W5 review, R40)', async () => {
    const onSave = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    render(<SegmentTextEditor initialText="original" onSave={onSave} onCancel={vi.fn()} />);

    const textarea = screen.getByRole('textbox');
    fireEvent.change(textarea, { target: { value: 'first attempt' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));

    // Nothing reset the field back to the original text on failure.
    expect(textarea).toHaveValue('first attempt');

    // The guard that stops a duplicate save released, so a retry goes through.
    fireEvent.keyDown(textarea, { key: 'Enter' });
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(2));
    expect(onSave).toHaveBeenLastCalledWith('first attempt');
  });

  it('uses the transcript text class (u-typed)', () => {
    render(<SegmentTextEditor initialText="x" onSave={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole('textbox')).toHaveClass('u-typed');
  });
});
