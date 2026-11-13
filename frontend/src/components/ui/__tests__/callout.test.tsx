import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Callout } from '@/components/ui/callout';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';

describe('Callout', () => {
  it.each([
    ['info', ['border-brand-edge', 'bg-brand-wash']],
    ['warn', ['border-brand', 'bg-brand-wash']],
    ['error', ['border-destructive-edge', 'bg-destructive-wash']],
    ['muted', ['border-border', 'bg-muted']],
    ['success', ['border-success-edge', 'bg-success-wash']],
  ] as const)('tone %s', (tone, classes) => {
    render(<Callout tone={tone}>note</Callout>);
    const box = screen.getByText('note');
    expect(box).toHaveClass('rounded-lg', 'border', 'px-4', 'py-3', ...classes);
  });

  it('never uses the REC red for a warning', () => {
    render(<Callout tone="warn">careful</Callout>);
    expect(screen.getByText('careful').className).not.toMatch(/record/);
  });

  it('announces errors only', () => {
    const { rerender } = render(<Callout tone="error">failed</Callout>);
    expect(screen.getByRole('alert')).toHaveTextContent('failed');
    rerender(<Callout tone="info">fyi</Callout>);
    expect(screen.queryByRole('alert')).toBeNull();
    rerender(<Callout tone="info" role="status">syncing</Callout>);
    expect(screen.getByRole('status')).toHaveTextContent('syncing');
  });

  it('lays out title, body and action', () => {
    render(
      <Callout tone="warn" title="Google Calendar needs reconnecting" action={<button type="button">Reconnect</button>}>
        Nothing has synced since your session expired.
      </Callout>,
    );
    expect(screen.getByText('Google Calendar needs reconnecting')).toHaveClass('font-medium');
    expect(screen.getByText(/Nothing has synced/)).toHaveClass('text-muted-foreground');
    expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument();
  });

  it('renders bare children straight into the box without title or action', () => {
    const { container } = render(
      <Callout>
        <svg data-testid="icon" />
        <p>text</p>
      </Callout>,
    );
    expect(screen.getByTestId('icon').parentElement).toBe(container.firstElementChild);
  });
});

describe('Alert (shim over Callout)', () => {
  it('keeps the plain bordered page-background box and role=alert', () => {
    render(
      <Alert>
        <AlertTitle>Heads up</AlertTitle>
        <AlertDescription>Body</AlertDescription>
      </Alert>,
    );
    const box = screen.getByRole('alert');
    expect(box).toHaveClass('bg-background', 'border-border', 'rounded-lg', 'px-4', 'py-3');
    expect(box).not.toHaveClass('bg-muted');
    expect(screen.getByText('Heads up').tagName).toBe('H5');
  });

  it("lets the caller's tint win", () => {
    render(<Alert className="border-brand/50 bg-brand/10">x</Alert>);
    const box = screen.getByRole('alert');
    expect(box).toHaveClass('bg-brand/10', 'border-brand/50');
    expect(box).not.toHaveClass('bg-background');
  });

  it('destructive is the error tone', () => {
    render(<Alert variant="destructive">bad</Alert>);
    expect(screen.getByRole('alert')).toHaveClass('bg-destructive-wash', 'text-destructive');
  });

  it('keeps the leading-svg icon layout', () => {
    render(<Alert>x</Alert>);
    expect(screen.getByRole('alert').className).toContain('[&>svg]:absolute');
  });
});

describe('Callout size and lead', () => {
  it('defaults to px-4 py-3 text-sm', () => {
    render(<Callout>Body</Callout>);
    const cls = screen.getByText('Body').className;
    expect(cls).toContain('px-4');
    expect(cls).toContain('py-3');
    expect(cls).toContain('text-sm');
  });

  it('has a compact px-3 py-2 text-xs size', () => {
    render(<Callout size="compact">Body</Callout>);
    const cls = screen.getByText('Body').className;
    expect(cls).toContain('px-3');
    expect(cls).toContain('py-2');
    expect(cls).toContain('text-xs');
    expect(cls).not.toContain('px-4');
    expect(cls).not.toContain('text-sm');
  });

  it('renders a lead before the content', () => {
    const { container } = render(
      <Callout size="compact" lead={<span data-testid="spin" />}>
        Working
      </Callout>,
    );
    const lead = container.querySelector('[data-callout-lead]');
    expect(lead).not.toBeNull();
    expect(lead?.contains(screen.getByTestId('spin'))).toBe(true);
    // The lead comes first, then the body.
    expect(lead?.nextElementSibling?.textContent).toBe('Working');
  });
});
