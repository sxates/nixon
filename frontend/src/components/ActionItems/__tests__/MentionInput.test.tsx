import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { MentionInput } from '../MentionInput';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
import { invoke } from '@tauri-apps/api/core';

const person = (id: string, displayName: string) => ({
  id, displayName, email: null, role: null, notes: null, voiceprintOptOut: false,
  starred: false, createdAt: '', updatedAt: '',
});

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation((async (cmd: string) => {
    if (cmd === 'api_list_people_ranked') return [person('p-1', 'Sam Rivera'), person('p-2', 'Sasha Lin'), person('p-3', 'Dana Cole')];
    throw new Error(`Unexpected ${cmd}`);
  }) as unknown as typeof invoke);
});

async function setup(props: Partial<React.ComponentProps<typeof MentionInput>> = {}) {
  const onSubmit = vi.fn();
  const onCancel = vi.fn();
  render(<MentionInput value="" onSubmit={onSubmit} onCancel={onCancel} placeholder="Add" {...props} />);
  await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalled());
  await Promise.resolve();
  const input = screen.getByLabelText('Add') as HTMLInputElement;
  return { input, onSubmit, onCancel };
}
const type = (input: HTMLInputElement, v: string) => fireEvent.change(input, { target: { value: v } });

describe('MentionInput', () => {
  it('typing @sa shows matching people under a People heading', async () => {
    const { input } = await setup();
    type(input, 'Check in with @sa');
    expect(await screen.findByText('People')).toBeTruthy();
    expect(screen.getByText('Sam Rivera')).toBeTruthy();
    expect(screen.getByText('Sasha Lin')).toBeTruthy();
    expect(screen.queryByText('Dana Cole')).toBeNull();
  });

  it('enter inserts mention and submit serializes token', async () => {
    const { input, onSubmit } = await setup();
    type(input, 'Check in with @sa');
    await screen.findByText('Sam Rivera');
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input.value).toBe('Check in with @Sam Rivera ');
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSubmit).toHaveBeenCalledWith('Check in with @[Sam Rivera](person:p-1) ');
  });

  it('arrow keys move the highlight', async () => {
    const { input, onSubmit } = await setup();
    type(input, '@sa');
    await screen.findByText('Sasha Lin');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSubmit).toHaveBeenCalledWith('@[Sasha Lin](person:p-2) ');
  });

  it('mouse pick works', async () => {
    const { input } = await setup();
    type(input, '@dan');
    fireEvent.click(await screen.findByText('Dana Cole'));
    expect(input.value).toBe('@Dana Cole ');
  });

  it('unpicked @word stays plain', async () => {
    const { input, onSubmit } = await setup();
    type(input, 'Ping @zed about it');
    expect(screen.queryByText('People')).toBeNull();
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSubmit).toHaveBeenCalledWith('Ping @zed about it');
  });

  it('escape closes the picker only, then cancels', async () => {
    const { input, onCancel } = await setup();
    type(input, '@sa');
    await screen.findByText('Sam Rivera');
    fireEvent.keyDown(input, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByText('Sam Rivera')).toBeNull());
    expect(onCancel).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('a space with no match closes the picker', async () => {
    const { input } = await setup();
    type(input, '@sa');
    await screen.findByText('Sam Rivera');
    type(input, '@sax ');
    await waitFor(() => expect(screen.queryByText('Sam Rivera')).toBeNull());
  });

  it('edit round-trips an existing token', async () => {
    const { input, onSubmit } = await setup({ value: 'Check in with @[Sam Rivera](person:p-1) (Friday)' });
    expect(input.value).toBe('Check in with @Sam Rivera (Friday)');
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSubmit).toHaveBeenCalledWith('Check in with @[Sam Rivera](person:p-1) (Friday)');
  });

  it('exposes combobox/listbox/option semantics', async () => {
    const { input } = await setup();
    expect(input.getAttribute('role')).toBe('combobox');
    expect(input.getAttribute('aria-expanded')).toBe('false');
    type(input, '@sa');
    await screen.findByText('Sam Rivera');
    expect(input.getAttribute('aria-expanded')).toBe('true');
    const list = screen.getByRole('listbox');
    expect(input.getAttribute('aria-controls')).toBe(list.id);
    const options = screen.getAllByRole('option');
    expect(options).toHaveLength(2);
    expect(input.getAttribute('aria-activedescendant')).toBe(options[0].id);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(input.getAttribute('aria-activedescendant')).toBe(options[1].id);
  });

  it('ignores Enter while an IME composition is in progress', async () => {
    const { input, onSubmit } = await setup();
    type(input, '@sa');
    await screen.findByText('Sam Rivera');
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 });
    expect(input.value).toBe('@sa');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('closes on blur and on Tab', async () => {
    const { input } = await setup();
    type(input, '@sa');
    await screen.findByText('Sam Rivera');
    fireEvent.blur(input);
    await waitFor(() => expect(screen.queryByText('Sam Rivera')).toBeNull());
    type(input, '@sam');
    await screen.findByText('Sam Rivera');
    fireEvent.keyDown(input, { key: 'Tab' });
    await waitFor(() => expect(screen.queryByText('Sam Rivera')).toBeNull());
  });

  it('Enter then blur submits once; Esc then blur never submits', async () => {
    const { input, onSubmit } = await setup({ submitOnBlur: true });
    type(input, 'Hello');
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.blur(input);
    expect(onSubmit).toHaveBeenCalledTimes(1);
    fireEvent.focus(input);
    type(input, 'Changed');
    fireEvent.keyDown(input, { key: 'Escape' });
    fireEvent.blur(input);
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('shows a renamed person by their current name', async () => {
    const { input } = await setup({ value: 'Ask @[Sammy](person:p-1)' });
    await waitFor(() => expect(input.value).toBe('Ask @Sam Rivera'));
  });
});

describe('MentionInput multiline', () => {
  it('sets the textarea height from scrollHeight so a long title wraps instead of clipping', async () => {
    const spy = vi.spyOn(HTMLTextAreaElement.prototype, 'scrollHeight', 'get').mockReturnValue(96);
    try {
      const { input } = await setup({ multiline: true, value: 'A long title that wraps' });
      expect(input.getAttribute('rows')).toBe('1');
      expect(input.style.height).toBe('96px');
      spy.mockReturnValue(140);
      type(input, 'A long title that wraps over a good many more words than it did');
      expect(input.style.height).toBe('140px');
    } finally {
      spy.mockRestore();
    }
  });
});
