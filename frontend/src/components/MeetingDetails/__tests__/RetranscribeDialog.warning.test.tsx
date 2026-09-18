import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { RetranscribeDialog } from '../RetranscribeDialog';

// specs/0061 W5 (task 5) — before a re-transcribe silently discards manual text
// corrections, warn the owner. The dialog reads the count on open
// (api_count_user_edited) and, when it's > 0, shows the exact warning copy in
// DialogDescription. Model/language plumbing (useTranscriptionModels, ConfigContext)
// is unrelated to this behavior and stubbed out.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/contexts/ConfigContext', () => ({
    useConfig: () => ({
        selectedLanguage: 'auto',
        transcriptModelConfig: { provider: 'parakeet', model: 'test-model' },
    }),
}));
vi.mock('@/hooks/useTranscriptionModels', () => ({
    useTranscriptionModels: () => ({
        availableModels: [],
        selectedModelKey: '',
        setSelectedModelKey: vi.fn(),
        loadingModels: false,
        fetchModels: vi.fn(),
        resetSelection: vi.fn(),
    }),
}));

function renderDialog() {
    return render(
        <RetranscribeDialog
            open={true}
            onOpenChange={vi.fn()}
            meetingId="meeting-1"
            meetingFolderPath="/recordings/meeting-1"
        />,
    );
}

describe('RetranscribeDialog — edited-lines warning (specs/0061 W5)', () => {
    beforeEach(() => {
        invoke.mockReset();
    });

    it('shows the exact warning copy when the meeting has user-edited lines', async () => {
        invoke.mockImplementation((cmd: string) => {
            if (cmd === 'api_count_user_edited') return Promise.resolve(3);
            return Promise.resolve(undefined);
        });

        renderDialog();

        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith('api_count_user_edited', { meetingId: 'meeting-1' }),
        );
        expect(
            await screen.findByText('Your 3 text edits will be replaced by the new transcription.'),
        ).toBeInTheDocument();
    });

    it('shows the normal description when there are no edited lines', async () => {
        invoke.mockImplementation((cmd: string) => {
            if (cmd === 'api_count_user_edited') return Promise.resolve(0);
            return Promise.resolve(undefined);
        });

        renderDialog();

        await waitFor(() =>
            expect(invoke).toHaveBeenCalledWith('api_count_user_edited', { meetingId: 'meeting-1' }),
        );
        expect(screen.queryByText(/text edits will be replaced/)).not.toBeInTheDocument();
        expect(
            screen.getByText('Re-process the audio with different language settings'),
        ).toBeInTheDocument();
    });
});
