import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock('@/components/AskAI/AnswerMarkdown', () => ({
  AnswerMarkdown: ({ markdown }: { markdown: string }) => <div>{markdown}</div>,
}));

import { UpdatedNotice } from '@/components/Updates/UpdatedNotice';

describe('UpdatedNotice (specs/0069 W6)', () => {
  beforeEach(() => invoke.mockReset());

  it('says what you got when a receipt comes back', async () => {
    invoke.mockResolvedValue({
      version: '0.8.0',
      notes: '### Fixed\n\n- The thing',
      installed_at: '2026-09-20T10:00:00Z',
    });
    render(<UpdatedNotice />);
    expect(await screen.findByText(/nixon updated to v0\.8\.0/i)).toBeInTheDocument();
    expect(screen.getByText(/the thing/i)).toBeInTheDocument();
  });

  it('renders nothing when there is no receipt', async () => {
    invoke.mockResolvedValue(null);
    const { container } = render(<UpdatedNotice />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_take_update_receipt'));
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it('survives a backend that is not there', async () => {
    invoke.mockRejectedValueOnce(new Error('not in tauri'));
    const { container } = render(<UpdatedNotice />);
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it('ignores a malformed payload instead of throwing (specs/0069 task 8)', async () => {
    // This is the honest case: a shots-mock fixture gap once made a real invoke resolve
    // an array here, which is truthy, and crashed the whole app shell on `.notes.trim()`.
    invoke.mockResolvedValue([]);
    const { container } = render(<UpdatedNotice />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('api_take_update_receipt'));
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });
});
