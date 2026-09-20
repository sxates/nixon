import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const request = vi.fn();
let canRestart = true;
let isRecording = false;
// specs/0069 W5 review gap: nothing tested that this component calls `request()` and never
// `install()` directly — it is the component all three UI paths (sidebar row, collapsed
// flyout, Settings > About) share, so a future edit that reached for `install()` here would
// bypass the confirmation everywhere at once and nothing would catch it. Mocking
// UpdateStatusContext's `install` here means such a regression shows up even though this
// component does not import that context today.
const install = vi.fn(async () => {});

vi.mock('@/contexts/RestartConfirmContext', () => ({ useRestartConfirm: () => ({ request, canRestart }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording }) }));
vi.mock('@/contexts/UpdateStatusContext', () => ({ useOptionalUpdateStatus: () => ({ install }) }));

import { RestartToUpdateButton } from '@/components/Updates/RestartToUpdateButton';

describe('RestartToUpdateButton (specs/0069 W5)', () => {
  beforeEach(() => {
    request.mockClear();
    install.mockClear();
    canRestart = true;
    isRecording = false;
  });

  it('clicking asks for confirmation and never installs directly', () => {
    render(<RestartToUpdateButton />);
    fireEvent.click(screen.getByRole('button', { name: 'Restart' }));
    expect(request).toHaveBeenCalledTimes(1);
    expect(install).not.toHaveBeenCalled();
  });

  it('renders the given label', () => {
    render(<RestartToUpdateButton label="Restart to update" />);
    expect(screen.getByRole('button', { name: 'Restart to update' })).toBeInTheDocument();
  });

  it('is disabled when the confirm context reports it cannot restart', () => {
    canRestart = false;
    render(<RestartToUpdateButton />);
    expect(screen.getByRole('button')).toBeDisabled();
  });

  it('titles the disabled reason while recording', () => {
    isRecording = true;
    render(<RestartToUpdateButton />);
    expect(screen.getByRole('button')).toHaveAttribute('title', 'Finish the recording first');
  });
});
