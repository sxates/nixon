import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ThemeProvider, THEME_STORAGE_KEY } from '@/contexts/ThemeContext';
import { AppearanceSettings } from '@/components/AppearanceSettings';

beforeEach(() => {
  localStorage.clear();
  window.matchMedia = vi.fn().mockImplementation((q: string) => ({
    matches: false, media: q, addEventListener: vi.fn(), removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
});

describe('AppearanceSettings', () => {
  it('shows three options with System selected by default', () => {
    render(<ThemeProvider><AppearanceSettings /></ThemeProvider>);
    const group = screen.getByRole('radiogroup', { name: /appearance/i });
    expect(group).toBeTruthy();
    expect(screen.getByRole('radio', { name: /system/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /faceplate/i })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('radio', { name: /deck/i })).toHaveAttribute('aria-checked', 'false');
  });

  it('selecting Deck persists dark and applies the class', () => {
    render(<ThemeProvider><AppearanceSettings /></ThemeProvider>);
    fireEvent.click(screen.getByRole('radio', { name: /deck/i }));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(screen.getByRole('radio', { name: /deck/i })).toHaveAttribute('aria-checked', 'true');
  });

  it('arrow keys move and select within the group, with a roving tabIndex', () => {
    render(<ThemeProvider><AppearanceSettings /></ThemeProvider>);
    const system = screen.getByRole('radio', { name: /system/i });
    const faceplate = screen.getByRole('radio', { name: /faceplate/i });
    const deck = screen.getByRole('radio', { name: /deck/i });

    // One tab stop: only the selected position is tabbable.
    expect(system).toHaveAttribute('tabindex', '0');
    expect(faceplate).toHaveAttribute('tabindex', '-1');

    // System is last in the group, so ArrowRight wraps to Faceplate.
    system.focus();
    fireEvent.keyDown(system, { key: 'ArrowRight' });
    expect(faceplate).toHaveAttribute('aria-checked', 'true');
    expect(document.activeElement).toBe(faceplate);
    expect(faceplate).toHaveAttribute('tabindex', '0');
    expect(system).toHaveAttribute('tabindex', '-1');

    // ArrowLeft wraps back the other way.
    fireEvent.keyDown(faceplate, { key: 'ArrowLeft' });
    expect(system).toHaveAttribute('aria-checked', 'true');
    expect(document.activeElement).toBe(system);

    // ArrowDown behaves like ArrowRight (vertical arrows are part of the contract).
    fireEvent.keyDown(system, { key: 'ArrowDown' });
    fireEvent.keyDown(faceplate, { key: 'ArrowDown' });
    expect(deck).toHaveAttribute('aria-checked', 'true');
    expect(document.activeElement).toBe(deck);
  });
});
