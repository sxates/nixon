import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { PersonActionsMenu } from '@/components/People/PersonActionsMenu';

// The one person overflow menu, shared by the People row and the person page header.
describe('PersonActionsMenu', () => {
  it('offers Edit and Manage voice samples when the host passes them', async () => {
    const onEdit = vi.fn();
    render(
      <PersonActionsMenu
        voiceprintOptOut={false}
        onEdit={onEdit}
        onManageVoice={vi.fn()}
        onToggleVoiceOptOut={vi.fn()}
        onForget={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Person options' }));
    expect(screen.getByRole('menuitem', { name: 'Manage voice samples' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('menuitem', { name: 'Edit' }));
    expect(onEdit).toHaveBeenCalledOnce();
  });

  it('hides voice samples when the voice is not stored, and omits Edit on the person page', async () => {
    render(
      <PersonActionsMenu
        voiceprintOptOut
        onManageVoice={vi.fn()}
        onToggleVoiceOptOut={vi.fn()}
        onForget={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Person options' }));
    expect(screen.queryByRole('menuitem', { name: 'Edit' })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: 'Manage voice samples' })).toBeNull();
  });

  it('toggles the voice opt-out and draws Forget as destructive', async () => {
    const onToggle = vi.fn();
    const onForget = vi.fn();
    render(
      <PersonActionsMenu
        voiceprintOptOut={false}
        onToggleVoiceOptOut={onToggle}
        onForget={onForget}
      />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Person options' }));
    await userEvent.click(screen.getByRole('menuitem', { name: /Don.t store this person.s voice/ }));
    expect(onToggle).toHaveBeenCalledOnce();
    const forget = screen.getByRole('menuitem', { name: 'Forget this person' });
    expect(forget.getAttribute('data-variant')).toBe('destructive');
    await userEvent.click(forget);
    expect(onForget).toHaveBeenCalledOnce();
  });
});
