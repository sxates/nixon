import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { Checkbox } from '@/components/ui/checkbox';

describe('Checkbox', () => {
  it('is a checkbox named by aria-label, unchecked by default', () => {
    render(<Checkbox aria-label="Select this line" />);
    const el = screen.getByRole('checkbox', { name: 'Select this line' });
    expect(el).not.toBeChecked();
    expect(el.getAttribute('data-state')).toBe('unchecked');
  });

  it('uncontrolled: toggles on click and reports the new value', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox aria-label="Work" onCheckedChange={onCheckedChange} />);
    const el = screen.getByRole('checkbox');
    fireEvent.click(el);
    expect(onCheckedChange).toHaveBeenCalledWith(true);
    expect(el).toBeChecked();
    fireEvent.click(el);
    expect(onCheckedChange).toHaveBeenLastCalledWith(false);
    expect(el).not.toBeChecked();
  });

  it('controlled: follows `checked`, reports the requested value, and does not flip itself', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox aria-label="Work" checked onCheckedChange={onCheckedChange} />);
    const el = screen.getByRole('checkbox');
    fireEvent.click(el);
    expect(onCheckedChange).toHaveBeenCalledWith(false);
    expect(el).toBeChecked();
  });

  it('`label` names the box, and clicking the text toggles it', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox label="Save to my prompt library" onCheckedChange={onCheckedChange} />);
    const el = screen.getByRole('checkbox', { name: 'Save to my prompt library' });
    fireEvent.click(screen.getByText('Save to my prompt library'));
    expect(onCheckedChange).toHaveBeenCalledTimes(1);
    expect(el).toBeChecked();
  });

  it('passes the click event to onClick first (shift-click range selection)', () => {
    const onClick = vi.fn();
    render(<Checkbox aria-label="Line" checked={false} onClick={onClick} />);
    fireEvent.click(screen.getByRole('checkbox'), { shiftKey: true });
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(onClick.mock.calls[0][0].shiftKey).toBe(true);
  });

  it('disabled does not toggle', () => {
    const onCheckedChange = vi.fn();
    render(<Checkbox aria-label="Work" disabled onCheckedChange={onCheckedChange} />);
    fireEvent.click(screen.getByRole('checkbox'));
    expect(onCheckedChange).not.toHaveBeenCalled();
  });

  it('uses the brand fill when checked and the focus ring', () => {
    render(<Checkbox aria-label="Work" defaultChecked />);
    const el = screen.getByRole('checkbox');
    expect(el.className).toContain('bg-brand');
    expect(el.className).toContain('rounded-sm');
    expect(el.className).toContain('focus-visible:ring-2');
  });
});
