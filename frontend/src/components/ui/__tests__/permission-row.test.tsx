import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { PermissionRow, PermissionStatus } from '@/components/ui/permission-row';

describe('PermissionStatus', () => {
  it.each([
    ['granted', 'Allowed', 'text-success', 'bg-success'],
    ['denied', 'Not allowed', 'text-destructive', 'bg-destructive'],
    ['silent', 'Not yet', 'text-muted-foreground', 'bg-engrave'],
  ] as const)('%s → a dot and "%s"', (state, word, text, dot) => {
    const { container } = render(<PermissionStatus state={state} />);
    const status = screen.getByRole('status');
    expect(status).toHaveTextContent(word);
    expect(status).toHaveClass(text);
    expect(container.querySelector('[data-tone]')).toHaveClass(dot, 'rounded-full');
    // A dot, never a check glyph.
    expect(container.querySelector('svg')).toBeNull();
  });

  it('checking shows a spinner', () => {
    const { container } = render(<PermissionStatus state="checking" />);
    expect(screen.getByRole('status')).toHaveTextContent('Checking…');
    expect(container.querySelector('svg.animate-spin')).not.toBeNull();
  });

  it('takes a label', () => {
    render(<PermissionStatus state="granted" label="On" />);
    expect(screen.getByRole('status')).toHaveTextContent('On');
  });
});

describe('PermissionRow', () => {
  it('title, description, status and action in one row', () => {
    render(
      <PermissionRow
        title="Microphone"
        description="Your side of the meeting."
        state="denied"
        action={<button type="button">Open System Settings</button>}
      />,
    );
    expect(screen.getByText('Microphone')).toHaveClass('text-sm', 'font-medium');
    expect(screen.getByText('Your side of the meeting.')).toHaveClass('text-muted-foreground');
    expect(screen.getByRole('status')).toHaveTextContent('Not allowed');
    expect(screen.getByRole('button', { name: 'Open System Settings' })).toBeInTheDocument();
  });

  it('card variant is a bordered card; row variant is flat', () => {
    const { container, rerender } = render(<PermissionRow title="Audio capture" state="granted" variant="card" />);
    expect(container.firstElementChild).toHaveClass('rounded-lg', 'border', 'bg-card');
    rerender(<PermissionRow title="Audio capture" state="granted" />);
    expect(container.firstElementChild).not.toHaveClass('border');
    expect(container.firstElementChild).toHaveAttribute('data-state', 'granted');
  });
});
