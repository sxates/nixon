import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// specs/0039 WS3 (task 9) — per-sample voice gallery management on People detail.
// These lock: (a) the list renders each sample with a live/quarantined badge, (b) Quarantine /
// Restore dispatch the recoverable commands, and (c) Delete is PERMANENT + confirm-gated — the
// trash button alone must not fire api_delete_voiceprint_sample; only the confirm does.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';
import { VoiceprintSamplesDialog } from '@/components/People/VoiceprintSamplesDialog';
import { VoiceprintSamplesList } from '@/components/People/VoiceprintSamplesList';
import type { VoiceprintSampleDto } from '@/types';

const invokeMock = vi.mocked(invoke);

const SAMPLES: VoiceprintSampleDto[] = [
  {
    id: 'vp-active',
    sourceMeetingId: 'm-1',
    sourceSpeakerKey: 'spk_0',
    createdAt: '2026-06-24T10:00:00Z',
    sampleQuality: 0.9,
    quarantined: false,
  },
  {
    id: 'vp-quar',
    sourceMeetingId: 'm-1',
    sourceSpeakerKey: 'spk_1',
    createdAt: '2026-06-20T10:00:00Z',
    sampleQuality: null,
    quarantined: true,
  },
  {
    // Legacy sample: no source speaker key — still fully manageable, never hidden.
    id: 'vp-legacy',
    sourceMeetingId: null,
    sourceSpeakerKey: null,
    createdAt: '2026-06-10T10:00:00Z',
    sampleQuality: 0.5,
    quarantined: false,
  },
];

function routeInvoke(samples: VoiceprintSampleDto[] = SAMPLES) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_list_person_voiceprints':
        return Promise.resolve(samples);
      case 'api_get_meeting_metadata':
        return Promise.resolve({ id: 'm-1', title: 'Weekly sync' });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function renderDialog() {
  return render(
    <VoiceprintSamplesDialog
      open
      onOpenChange={vi.fn()}
      personId="p-1"
      personName="Priya Patel"
    />,
  );
}

describe('VoiceprintSamplesDialog (specs/0039 WS3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists each stored sample with a live/quarantined breakdown', async () => {
    routeInvoke();
    renderDialog();

    // Loads via api_list_person_voiceprints keyed by personId.
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_list_person_voiceprints', { personId: 'p-1' }),
    );

    // 2 active (source + legacy), 1 quarantined.
    expect(await screen.findByText('2 active · 1 quarantined')).toBeInTheDocument();
    // One quarantined badge, at least one active badge.
    expect(screen.getByText('Quarantined')).toBeInTheDocument();
    expect(screen.getAllByText('Active').length).toBeGreaterThanOrEqual(1);
    // Resolved source-meeting title surfaces (both m-1 samples reference it).
    expect(screen.getAllByText(/Weekly sync/).length).toBeGreaterThanOrEqual(1);
  });

  it('quarantines an active sample via api_quarantine_voiceprint_sample', async () => {
    routeInvoke();
    renderDialog();
    await screen.findByText('2 active · 1 quarantined');

    // Two active samples ⇒ two Quarantine buttons; click the first.
    const quarantineButtons = screen.getAllByRole('button', { name: /Quarantine/ });
    fireEvent.click(quarantineButtons[0]!);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_quarantine_voiceprint_sample', {
        sampleId: 'vp-active',
      }),
    );
  });

  it('restores a quarantined sample via api_restore_voiceprint_sample', async () => {
    routeInvoke();
    renderDialog();
    await screen.findByText('2 active · 1 quarantined');

    fireEvent.click(screen.getByRole('button', { name: /Restore/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_restore_voiceprint_sample', {
        sampleId: 'vp-quar',
      }),
    );
  });

  it('gates permanent delete behind an inline confirm', async () => {
    routeInvoke();
    renderDialog();
    await screen.findByText('2 active · 1 quarantined');

    // The trash affordance opens the confirm — it must NOT delete on its own.
    const trashButtons = screen.getAllByRole('button', {
      name: /Delete this voice sample permanently/,
    });
    fireEvent.click(trashButtons[0]!);

    expect(
      await screen.findByText(/Delete this sample forever\? This can't be undone\./),
    ).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('api_delete_voiceprint_sample', expect.anything());

    // Confirming actually deletes.
    fireEvent.click(screen.getByRole('button', { name: /^Delete$/ }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_delete_voiceprint_sample', {
        sampleId: 'vp-active',
      }),
    );
  });

  it('shows an empty state when the person has no samples', async () => {
    routeInvoke([]);
    renderDialog();
    expect(await screen.findByText('No voice samples yet')).toBeInTheDocument();
  });

  // specs/0056 W5 — the dialog keeps a bounded, natively scrolling list; the person page's
  // "Voice Samples" tab renders the same list UNBOUNDED so the page's own scroll container
  // scrolls it like the Summary / Recent-meetings tabs (the old Radix ScrollArea with only a
  // max-height never overflowed, so the tab clipped at 52vh and did not scroll at all).
  it('dialog list is bounded and scrolls natively (overflow-y-auto)', async () => {
    routeInvoke();
    renderDialog();
    await screen.findByText('2 active · 1 quarantined');

    const scroller = screen.getByRole('list').parentElement!;
    expect(scroller.className).toMatch(/overflow-y-auto/);
    expect(scroller.className).toMatch(/max-h-/);
  });

  it('unbounded list (person page tab) has no max-height so the page scrolls it', async () => {
    routeInvoke();
    render(<VoiceprintSamplesList personId="p-1" personName="Priya Patel" />);
    await screen.findByText('2 active · 1 quarantined');

    const wrapper = screen.getByRole('list').parentElement!;
    expect(wrapper.className).not.toMatch(/overflow-y-auto/);
    // No ancestor may cap the height either — the old ScrollArea root carried max-h-[52vh].
    for (let el: HTMLElement | null = wrapper; el; el = el.parentElement) {
      expect(el.className).not.toMatch(/max-h-/);
    }
  });
});
