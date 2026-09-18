import { toast } from 'sonner';

/**
 * specs/0057 Plan 2 Task 7 — user-facing copy for a failed recording start.
 *
 * Extracted verbatim from the retired `RecordingControls.tsx` (:120-146), which owned both
 * the mapping and the alert it rendered. The control surface is gone (the transport rail
 * owns REC/HOLD/STOP now), but the copy is the contract the user sees, so it lives on here
 * as a pure function the start hook can route into the existing error modal.
 */
export interface RecordingStartErrorCopy {
  title: string;
  message: string;
}

/** Best-effort message text out of whatever the backend/IPC layer threw. */
function messageOf(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  if (err && typeof err === 'object' && 'message' in err) {
    const m = (err as { message?: unknown }).message;
    if (typeof m === 'string') return m;
  }
  return err == null ? '' : String(err);
}

export function describeRecordingStartError(err: unknown): RecordingStartErrorCopy {
  const errorMsg = messageOf(err);

  // Branch order is load-bearing: a message naming both the microphone and system audio
  // reports the microphone, exactly as the original did.
  if (errorMsg.includes('microphone') || errorMsg.includes('mic') || errorMsg.includes('input')) {
    return {
      title: 'Microphone Not Available',
      message:
        'Unable to access your microphone. Please check that:\n• Your microphone is connected\n• The app has microphone permissions\n• No other app is using the microphone',
    };
  }
  if (errorMsg.includes('system audio') || errorMsg.includes('speaker') || errorMsg.includes('output')) {
    return {
      title: 'System Audio Not Available',
      message:
        'Unable to capture system audio. Please check that:\n• A virtual audio device (like BlackHole) is installed\n• The app has audio-capture permissions (macOS)\n• System audio is properly configured',
    };
  }
  if (errorMsg.includes('permission')) {
    return {
      title: 'Permission Required',
      message:
        'Recording permissions are required. Please:\n• Grant microphone access in System Settings\n• Grant audio-capture access for system audio (macOS)\n• Restart the app after granting permissions',
    };
  }
  return {
    title: 'Recording Failed',
    message: 'Unable to start recording. Please check your audio device settings and try again.',
  };
}

/**
 * Show a failed start to the user. The device-specific copy used to be rendered as an inline
 * alert by RecordingControls; with the controls gone the start hook routes it into the page's
 * error modal instead (and falls back to a toast when no modal host is mounted).
 */
export function surfaceRecordingStartFailure(
  error: unknown,
  showModal?: (name: 'errorAlert', message?: string) => void,
): void {
  const { title, message } = describeRecordingStartError(error);
  if (showModal) showModal('errorAlert', `${title}\n${message}`);
  else toast.error(title, { description: message });
}
