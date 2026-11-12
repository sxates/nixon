import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';

describe('Label', () => {
  it('appends a muted "(optional)" when optional', () => {
    render(<Label htmlFor="x" optional>Email</Label>);
    const hint = screen.getByText('(optional)');
    expect(hint.className).toContain('text-muted-foreground');
    expect(hint.closest('label')!.textContent).toBe('Email(optional)');
  });

  it('renders no hint by default', () => {
    render(<Label>Email</Label>);
    expect(screen.queryByText('(optional)')).toBeNull();
  });
});

describe('Input / Textarea focus', () => {
  it('use the 2px focus ring like every other control', () => {
    render(
      <>
        <Input aria-label="Name" />
        <Textarea aria-label="Notes" />
      </>
    );
    for (const el of [screen.getByLabelText('Name'), screen.getByLabelText('Notes')]) {
      expect(el.className).toContain('focus-visible:ring-2');
      expect(el.className).not.toContain('focus-visible:ring-1');
    }
  });
});

describe('Input size', () => {
  it('is 36px at 14px by default and 36px at text-lead when lg', () => {
    render(
      <>
        <Input aria-label="Plain" />
        <Input aria-label="Question" size="lg" />
      </>,
    );
    const plain = screen.getByRole('textbox', { name: 'Plain' }).className;
    const lg = screen.getByRole('textbox', { name: 'Question' }).className;
    expect(plain).toContain('h-9');
    expect(plain).toContain('md:text-sm');
    expect(lg).toContain('h-9');
    expect(lg).toContain('md:text-lead');
    expect(lg).not.toContain('md:text-sm');
  });
});
