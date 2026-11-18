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

  it('Enter adds an empty row right after the current one and focuses it', () => {
    const e = entry({
      body: { bullets: [{ text: 'First.', t: 10 }, { text: 'Third.', t: 30 }], decisions: [] },
    });
    render(<EntryEditor topicId="topic-1" entry={e} onDone={vi.fn()} />);
    fireEvent.keyDown(screen.getByDisplayValue('First.'), { key: 'Enter' });
    const points = screen.getAllByRole('textbox', { name: /^Point \d$/ });
    expect(points.map((p) => (p as HTMLInputElement).value)).toEqual(['First.', '', 'Third.']);
    expect(points[1]).toHaveFocus();
    expect(editTopicEntry).not.toHaveBeenCalled();
  });

  it('Enter in a Decision adds a Decision row, not a Point', () => {
    const e = entry({ body: { bullets: [], decisions: [{ id: 'dec-1', text: 'Ship it.', t: null }] } });
    render(<EntryEditor topicId="topic-1" entry={e} onDone={vi.fn()} />);
    fireEvent.keyDown(screen.getByDisplayValue('Ship it.'), { key: 'Enter' });
    expect(screen.getByRole('textbox', { name: 'Decision 2' })).toHaveFocus();
    expect(screen.queryByRole('textbox', { name: /^Point/ })).toBeNull();
  });

  it('Backspace in an empty row removes it and focuses the row above', () => {
    render(<EntryEditor topicId="topic-1" onDone={vi.fn()} />);
    const first = screen.getByRole('textbox', { name: 'Point 1' });
    fireEvent.change(first, { target: { value: 'One.' } });
    fireEvent.keyDown(first, { key: 'Enter' });
    const second = screen.getByRole('textbox', { name: 'Point 2' });
    expect(second).toHaveFocus();
    fireEvent.keyDown(second, { key: 'Backspace' });
    expect(screen.queryByRole('textbox', { name: 'Point 2' })).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Point 1' })).toHaveFocus();
  });

  it('Backspace in a row with text, or in the first row, removes nothing', () => {
    render(<EntryEditor topicId="topic-1" onDone={vi.fn()} />);
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Point 1' }), { key: 'Backspace' });
    expect(screen.getByRole('textbox', { name: 'Point 1' })).toBeInTheDocument();
    fireEvent.change(screen.getByRole('textbox', { name: 'Point 1' }), { target: { value: 'Keep.' } });
    fireEvent.keyDown(screen.getByDisplayValue('Keep.'), { key: 'Enter' });
    fireEvent.change(screen.getByRole('textbox', { name: 'Point 2' }), { target: { value: 'x' } });
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Point 2' }), { key: 'Backspace' });
    expect(screen.getAllByRole('textbox', { name: /^Point/ })).toHaveLength(2);
  });

  it.each([
    ['⌘Enter', { metaKey: true }],
    ['Ctrl+Enter', { ctrlKey: true }],
  ])('%s saves the entry and adds no row', async (_name, mod) => {
    const onDone = vi.fn();
    render(<EntryEditor topicId="topic-1" onDone={onDone} />);
    const first = screen.getByRole('textbox', { name: 'Point 1' });
    fireEvent.change(first, { target: { value: 'Sent the deck.' } });
    fireEvent.keyDown(first, { key: 'Enter', ...mod });
    await waitFor(() =>
      expect(addManualTopicEntry).toHaveBeenCalledWith('topic-1', {
        bullets: [{ text: 'Sent the deck.', t: null }],
        decisions: [],
      }),
    );
    expect(onDone).toHaveBeenCalled();
    expect(screen.queryByRole('textbox', { name: 'Point 2' })).toBeNull();
  });

  it('⌘Enter on an empty entry saves nothing', () => {
    render(<EntryEditor topicId="topic-1" onDone={vi.fn()} />);
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Point 1' }), { key: 'Enter', metaKey: true });
    expect(addManualTopicEntry).not.toHaveBeenCalled();
  });

  it('shows the keyboard hint by the Save button', () => {
    render(<EntryEditor topicId="topic-1" onDone={vi.fn()} />);
    expect(screen.getByText('↵ new row · ⌘↵ save')).toBeInTheDocument();
  });
});
