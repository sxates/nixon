import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { RunStatus } from '@/components/ui/run-status';

describe('EmptyState', () => {
  it('renders title, body and action with no icon disc', () => {
    const { container } = render(
      <EmptyState
        title="No recordings yet"
        body="Start recording and your meetings will show up here."
        action={<button type="button">Open the recorder</button>}
      />,
    );
    expect(screen.getByRole('heading', { name: 'No recordings yet' })).toHaveClass('text-lead');
    expect(screen.getByText(/Start recording/)).toHaveClass('text-muted-foreground');
    expect(screen.getByRole('button', { name: 'Open the recorder' })).toBeInTheDocument();
    expect(container.querySelector('svg')).toBeNull();
    expect(container.querySelector('.rounded-full')).toBeNull();
  });

  it('panel variant is a dashed Panel', () => {
    const { container } = render(<EmptyState variant="panel" title="No brief yet" />);
    const root = container.firstElementChild!;
    expect(root).toHaveAttribute('data-empty-state', 'panel');
    expect(root).toHaveClass('border-dashed', 'text-center');
  });

  it('takes a heading level', () => {
    render(<EmptyState title="Nothing" titleAs="h3" />);
    expect(screen.getByRole('heading', { name: 'Nothing' }).tagName).toBe('H3');
  });
});

describe('PageLoading', () => {
  it('fills the page under the rail with a labelled status', () => {
    render(<PageLoading label="Loading meetings…" />);
    const status = screen.getByRole('status', { name: 'Loading meetings…' });
    expect(status).toHaveClass('h-page', 'items-center', 'justify-center');
    expect(status).toHaveTextContent('Loading meetings…');
    expect(status.querySelector('svg.animate-spin')).not.toBeNull();
  });

  it('pane fill and a visually hidden label', () => {
    render(<PageLoading fill="pane" label="Loading" hideLabel />);
    const status = screen.getByRole('status', { name: 'Loading' });
    expect(status).toHaveClass('h-64');
    expect(status).toHaveTextContent('');
  });

  it('is named "Loading" without a label', () => {
    render(<PageLoading />);
    expect(screen.getByRole('status', { name: 'Loading' })).toBeInTheDocument();
  });
});

describe('RunStatus', () => {
  it('shows the live label in a status region inside a Panel', () => {
    const { container } = render(<RunStatus label="Searching 12 meetings…" />);
    expect(screen.getByRole('status')).toHaveTextContent('Searching 12 meetings…');
    expect(container.firstElementChild).toHaveClass('rounded-lg', 'bg-card', 'shadow-card');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('offers Cancel and shows the cancelling state', () => {
    const onCancel = vi.fn();
    const { rerender } = render(<RunStatus label="Working" onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalledOnce();
    rerender(<RunStatus label="Working" onCancel={onCancel} cancelling />);
    expect(screen.getByRole('button', { name: 'Cancelling…' })).toBeDisabled();
  });
});
