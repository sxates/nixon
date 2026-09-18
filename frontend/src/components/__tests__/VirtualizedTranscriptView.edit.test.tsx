import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { TranscriptSegmentData } from '@/types';

// specs/0061 W5 (task 5) — inline segment text editing, in the mock-free, real-DOM
// style of the deep-link/span-reassign tests: the view only calls the `onEditText`
// prop it's handed. TranscriptPanel owns the actual invoke('api_set_segment_text', …)
// and is covered separately.

// "um" is stop-word-cleaned out of the DISPLAYED text — the editor must open with
// the RAW segment text (including "um"), never the cleaned display text.
const SEGMENTS: TranscriptSegmentData[] = [
    { id: 'seg-1', timestamp: 0, text: 'um the anodised and closure lead time' },
];

function view(onEditText?: (id: string, text: string) => Promise<boolean>) {
    return (
        <TooltipProvider>
            <VirtualizedTranscriptView segments={SEGMENTS} disableAutoScroll onEditText={onEditText} />
        </TooltipProvider>
    );
}

describe('VirtualizedTranscriptView — inline segment text editing (specs/0061 W5)', () => {
    it('does not render an edit affordance when onEditText is absent', () => {
        render(view(undefined));
        expect(screen.queryByRole('button', { name: /edit this line/i })).not.toBeInTheDocument();
    });

    it('hover pencil opens the editor with the RAW text, not the cleaned display text', () => {
        render(view(vi.fn()));

        // Sanity: the rendered paragraph shows the cleaned text (no "um").
        expect(screen.getByText('the anodised and closure lead time')).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: /edit this line/i }));

        expect(screen.getByRole('textbox', { name: /edit transcript line/i })).toHaveValue(
            'um the anodised and closure lead time',
        );
    });

    it('save calls onEditText and the row shows the new text and an edited mark', async () => {
        const onEditText = vi.fn().mockResolvedValue(true);
        render(view(onEditText));

        fireEvent.click(screen.getByRole('button', { name: /edit this line/i }));
        const textarea = screen.getByRole('textbox', { name: /edit transcript line/i });
        fireEvent.change(textarea, { target: { value: 'the anodized end closure lead time' } });
        fireEvent.keyDown(textarea, { key: 'Enter' });

        await waitFor(() =>
            expect(onEditText).toHaveBeenCalledWith('seg-1', 'the anodized end closure lead time'),
        );
        expect(await screen.findByText('the anodized end closure lead time')).toBeInTheDocument();
        expect(screen.getByText('edited')).toBeInTheDocument();
    });

    it('reverts to the original text when the save fails', async () => {
        const onEditText = vi.fn().mockResolvedValue(false);
        render(view(onEditText));

        fireEvent.click(screen.getByRole('button', { name: /edit this line/i }));
        const textarea = screen.getByRole('textbox', { name: /edit transcript line/i });
        fireEvent.change(textarea, { target: { value: 'a rejected edit' } });
        fireEvent.keyDown(textarea, { key: 'Enter' });

        await waitFor(() => expect(onEditText).toHaveBeenCalled());
        expect(await screen.findByText('the anodised and closure lead time')).toBeInTheDocument();
        expect(screen.queryByText('a rejected edit')).not.toBeInTheDocument();
        expect(screen.queryByText('edited')).not.toBeInTheDocument();
    });
});
