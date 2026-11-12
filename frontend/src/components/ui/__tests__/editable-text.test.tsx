import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { EditableText } from '@/components/ui/editable-text';

const noop = () => {};

describe('EditableText', () => {
  it('shows the text with a pencil that starts editing', () => {
    const onStartEditing = vi.fn();
    render(
      <EditableText title="Key points" isEditing={false} onStartEditing={onStartEditing} onFinishEditing={noop} onChange={noop} />
    );
    expect(screen.getByText('Key points')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Edit section title' }));
    expect(onStartEditing).toHaveBeenCalledOnce();
  });

  it('shows a delete button only when onDelete is given', () => {
    const { rerender } = render(
      <EditableText title="t" isEditing={false} onStartEditing={noop} onFinishEditing={noop} onChange={noop} />
    );
    expect(screen.queryByRole('button', { name: 'Delete section' })).toBeNull();
    const onDelete = vi.fn();
    rerender(
      <EditableText title="t" isEditing={false} onStartEditing={noop} onFinishEditing={noop} onChange={noop} onDelete={onDelete} />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete section' }));
    expect(onDelete).toHaveBeenCalledOnce();
  });

  it('commits on Enter but keeps Shift+Enter for a newline', () => {
    const onFinishEditing = vi.fn();
    const onChange = vi.fn();
    render(
      <EditableText title="t" isEditing onStartEditing={noop} onFinishEditing={onFinishEditing} onChange={onChange} />
    );
    const box = screen.getByRole('textbox');
    fireEvent.change(box, { target: { value: 'New' } });
    expect(onChange).toHaveBeenCalledWith('New');
    fireEvent.keyDown(box, { key: 'Enter', shiftKey: true });
    expect(onFinishEditing).not.toHaveBeenCalled();
    fireEvent.keyDown(box, { key: 'Enter' });
    expect(onFinishEditing).toHaveBeenCalledOnce();
  });
});
