import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
const editTopicEntry = vi.fn();
const addManualTopicEntry = vi.fn();
vi.mock('@/lib/topics', async (orig) => ({
  ...(await orig<typeof import('@/lib/topics')>()),
  editTopicEntry: (...a: unknown[]) => editTopicEntry(...a),
  addManualTopicEntry: (...a: unknown[]) => addManualTopicEntry(...a),
}));

import { EntryEditor } from '@/components/Topics/EntryEditor';
import { entry } from './fixtures';

describe('EntryEditor', () => {
  beforeEach(() => {
    editTopicEntry.mockReset().mockResolvedValue(undefined);
    addManualTopicEntry.mockReset().mockResolvedValue('te-new');
  });

  it('keeps each row time and decision id, drops removed rows, and saves an edit', async () => {
    const onDone = vi.fn();
    const e = entry({
      body: {
        bullets: [
          { text: 'Keep me.', t: 75 },
          { text: 'Remove me.', t: 90 },
        ],
        decisions: [{ id: 'dec-1', text: 'Migrate in two waves.', t: 312 }],
      },
    });
    render(<EntryEditor topicId="topic-1" entry={e} onDone={onDone} />);
    fireEvent.change(screen.getByDisplayValue('Keep me.'), { target: { value: 'Kept, reworded.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Remove point 2' }));
    fireEvent.change(screen.getByDisplayValue('Migrate in two waves.'), {
      target: { value: 'Migrate in three waves.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add decision' }));
    const inputs = screen.getAllByRole('textbox');
    fireEvent.change(inputs[inputs.length - 1], { target: { value: 'Pilot with support.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(editTopicEntry).toHaveBeenCalledWith('te-1', {
        bullets: [{ text: 'Kept, reworded.', t: 75 }],
        decisions: [
          { id: 'dec-1', text: 'Migrate in three waves.', t: 312 },
          { id: '', text: 'Pilot with support.', t: null },
        ],
      }),
    );
    expect(onDone).toHaveBeenCalled();
  });

  it('adds a manual entry when there is no entry', async () => {
    render(<EntryEditor topicId="topic-1" onDone={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    fireEvent.change(screen.getAllByRole('textbox')[0], { target: { value: 'Called the vendor.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(addManualTopicEntry).toHaveBeenCalledWith('topic-1', {
        bullets: [{ text: 'Called the vendor.', t: null }],
        decisions: [],
      }),
    );
    expect(editTopicEntry).not.toHaveBeenCalled();
  });
});
