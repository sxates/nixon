import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';

/**
 * Shows the recording notification toast with compliance message.
 * Checks user preferences and displays a dismissible toast with:
 * - notice to inform participants
 * - "Don't show again" checkbox
 * - Acknowledgment button
 *
 * @returns Promise<void> - Resolves when notification is shown or skipped
 */
export async function showRecordingNotification(): Promise<void> {
  try {
    const { Store } = await import('@tauri-apps/plugin-store');
    const store = await Store.load('preferences.json');
    const showNotification = await store.get<boolean>('show_recording_notification') ?? true;

    if (showNotification) {
      let dontShowAgain = false;

      const toastId = toast.info('🔴 Recording Started', {
        description: (
          <div className="space-y-3 min-w-[280px]">
            <p className="text-sm font-medium text-foreground">
              Inform all participants this meeting is being recorded.
            </p>
            <Checkbox
              size="sm"
              label="Don't show this again"
              labelClassName="text-xs"
              onCheckedChange={(checked) => {
                dontShowAgain = checked;
              }}
            />
            <Button
              size="xs"
              className="w-full"
              onClick={async () => {
                if (dontShowAgain) {
                  const { Store } = await import('@tauri-apps/plugin-store');
                  const store = await Store.load('preferences.json');
                  await store.set('show_recording_notification', false);
                  await store.save();
                }
                toast.dismiss(toastId);
              }}
            >
              I&apos;ve Notified Participants
            </Button>
          </div>
        ),
        duration: 10000,
        position: 'bottom-right',
      });
    }
  } catch (notificationError) {
    console.error('Failed to show recording notification:', notificationError);
    // Don't fail the recording if notification fails
  }
}
