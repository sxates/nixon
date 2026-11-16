import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));

import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';

// specs/0090 — the explicit Topic control on the To-Do page's add row.

const TOPICS = [
  { id: 'atlas', name: 'Atlas CRM rebuild' },
  { id: 'lab', name: 'Lab equipment' },
];

describe('AddActionItemRow topic picker', () => {
  it('shows No Topic, picks a topic, passes it on add, then resets', async () => {
    const onAdd = vi.fn(async () => true);
    render(<AddActionItemRow ariaLabel="New To-Do" onAdd={onAdd} topics={TOPICS} topicPicker />);
    await userEvent.click(screen.getByRole('button', { name: 'Topic: No Topic' }));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Lab equipment' }));
    expect(screen.getByRole('button', { name: 'Topic: Lab equipment' })).toBeInTheDocument();

    const field = screen.getByLabelText('New To-Do');
    fireEvent.change(field, { target: { value: 'Book the calibration' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() => expect(onAdd).toHaveBeenCalledWith('Book the calibration', undefined, 'lab'));
    expect(await screen.findByRole('button', { name: 'Topic: No Topic' })).toBeInTheDocument();
  });

  it('No Topic in the menu clears a pick; without topicPicker there is no control', async () => {
    const { unmount } = render(<AddActionItemRow onAdd={vi.fn()} topics={TOPICS} topicPicker />);
    await userEvent.click(screen.getByRole('button', { name: 'Topic: No Topic' }));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Atlas CRM rebuild' }));
    await userEvent.click(screen.getByRole('button', { name: 'Topic: Atlas CRM rebuild' }));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'No Topic' }));
    expect(screen.getByRole('button', { name: 'Topic: No Topic' })).toBeInTheDocument();
    unmount();
    render(<AddActionItemRow onAdd={vi.fn()} topics={TOPICS} />);
    expect(screen.queryByRole('button', { name: /^Topic:/ })).toBeNull();
  });
});
