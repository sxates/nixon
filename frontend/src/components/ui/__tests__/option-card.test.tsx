import { describe, it, expect, vi } from 'vitest';
import { useState } from 'react';
import { render, screen, fireEvent } from '@testing-library/react';

import { OptionCard, OptionCardGroup } from '@/components/ui/option-card';

describe('OptionCard (standalone)', () => {
  it('is a radio with title, description and trailing', () => {
    const onSelect = vi.fn();
    render(
      <OptionCard
        title="Faceplate"
        description="Light"
        trailing={<span>142 MB</span>}
        selected={false}
        onSelect={onSelect}
      />,
    );
    const radio = screen.getByRole('radio', { name: /Faceplate/ });
    expect(radio).toHaveAttribute('aria-checked', 'false');
    expect(radio).toHaveClass('rounded-md', 'border-border', 'bg-panel', 'focus-visible:ring-2');
    expect(screen.getByText('142 MB')).toBeInTheDocument();
    fireEvent.click(radio);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('selected: brand edge, card face, lit dot', () => {
    const { container } = render(<OptionCard title="Deck" selected onSelect={() => {}} />);
    const radio = screen.getByRole('radio');
    expect(radio).toHaveAttribute('aria-checked', 'true');
    expect(radio).toHaveClass('border-brand', 'bg-card');
    expect(container.querySelector('[data-tone="brand"]')).not.toBeNull();
  });
});

function Themes({ onChange }: { onChange?: (v: string) => void }) {
  const [value, setValue] = useState('system');
  return (
    <OptionCardGroup
      aria-label="Theme"
      value={value}
      onValueChange={(v) => {
        setValue(v);
        onChange?.(v);
      }}
    >
      <OptionCard value="light" title="Faceplate" />
      <OptionCard value="dark" title="Deck" />
      <OptionCard value="system" title="System" />
    </OptionCardGroup>
  );
}

describe('OptionCardGroup', () => {
  it('is a named radiogroup with one Tab stop on the selected card', () => {
    render(<Themes />);
    expect(screen.getByRole('radiogroup', { name: 'Theme' })).toBeInTheDocument();
    const radios = screen.getAllByRole('radio');
    expect(radios.map((r) => r.getAttribute('tabindex'))).toEqual(['-1', '-1', '0']);
    expect(screen.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
  });

  it('click selects; arrows move focus and selection, wrapping', () => {
    const onChange = vi.fn();
    render(<Themes onChange={onChange} />);
    fireEvent.click(screen.getByRole('radio', { name: 'Faceplate' }));
    expect(onChange).toHaveBeenLastCalledWith('light');
    const faceplate = screen.getByRole('radio', { name: 'Faceplate' });
    fireEvent.keyDown(faceplate, { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith('system');
    expect(document.activeElement).toBe(screen.getByRole('radio', { name: 'System' }));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(onChange).toHaveBeenLastCalledWith('light');
    expect(screen.getByRole('radio', { name: 'Faceplate' })).toHaveAttribute('tabindex', '0');
  });
});
