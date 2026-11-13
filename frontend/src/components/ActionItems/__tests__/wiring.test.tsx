import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { AddActionItemRow } from '../AddActionItemRow';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
import { invoke } from '@tauri-apps/api/core';

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((async () => [
    { id: 'p-1', displayName: 'Sam Rivera', email: null, role: null, notes: null, voiceprintOptOut: false, starred: false, createdAt: '', updatedAt: '' },
  ]) as unknown as typeof invoke);
});

describe('quick-add mentions', () => {
  it('sends the token and clears the field', async () => {
    const onAdd = vi.fn();
    render(<AddActionItemRow placeholder="Add" onAdd={onAdd} />);
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
    const input = screen.getByLabelText('Add') as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'Check in with @sa' } });
    fireEvent.click(await screen.findByText('Sam Rivera'));
    fireEvent.change(input, { target: { value: input.value } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(onAdd).toHaveBeenCalled());
    expect(onAdd.mock.calls[0][0]).toContain('@[Sam Rivera](person:p-1)');
    await waitFor(() => expect(input.value).toBe(''));
  });
});
