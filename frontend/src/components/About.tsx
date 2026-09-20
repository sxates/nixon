import React, { useState, useEffect } from "react";
import { getVersion } from '@tauri-apps/api/app';
import { useOptionalUpdateStatus, describeStatus } from '@/contexts/UpdateStatusContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { SettingsNote } from '@/components/ui/settings';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';

// specs/0058 Task 5 — the update status line + Check for updates / Restart to
// update controls. Provider-tolerant: renders nothing when the app-wide
// UpdateStatusProvider isn't mounted (e.g. this component's own unit test).
// useRecordingState() is only called from the child below, which is only
// rendered once the update context is known to exist, so About stays safe to
// render outside a RecordingStateProvider too.
function UpdatesBlock() {
    const updates = useOptionalUpdateStatus();
    if (!updates) return null;
    return <UpdatesBlockContent {...updates} />;
}

function UpdatesBlockContent({
    status,
    busy,
    error,
    checkNow,
    install,
}: NonNullable<ReturnType<typeof useOptionalUpdateStatus>>) {
    const { isRecording } = useRecordingState();
    const ready = status.state === 'ready';
    const notes = ready ? status.notes.trim() : '';
    return (
        <div className="space-y-2 text-left">
            <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-muted-foreground">{describeStatus(status)}</span>
                <div className="flex gap-2">
                    {ready && (
                        <button
                            type="button"
                            onClick={() => install()}
                            disabled={busy || isRecording}
                            title={isRecording ? 'Finish the recording first' : undefined}
                            className="rounded-[3px] border border-border bg-key px-2.5 py-1 text-xs text-foreground hover:bg-key/80 disabled:opacity-50"
                        >
                            Restart to update
                        </button>
                    )}
                    <button
                        type="button"
                        onClick={() => checkNow()}
                        disabled={busy || status.state === 'checking' || status.state === 'downloading'}
                        className="rounded-[3px] border border-border px-2.5 py-1 text-xs text-muted-foreground hover:bg-key disabled:opacity-50"
                    >
                        Check for updates
                    </button>
                </div>
            </div>
            {status.state === 'error' && <p className="text-xs text-muted-foreground">{status.message}</p>}
            {error && <p className="text-xs text-record-ink">{error}</p>}
            {/* The release body IS markdown — it is the changelog's Added/Changed/Fixed
                sections, published verbatim (CLAUDE.md). Splitting it into one bullet per
                line printed the `###` headings as bullets and dropped the structure
                entirely (specs/0066 W4). `AnswerMarkdown` already renders this vocabulary,
                so the notes go through it rather than through a second renderer or a new
                markdown dependency. */}
            {ready && notes.length > 0 && (
                <SettingsNote tone="muted">
                    <p className="u-section-label mb-1 text-[9px]">What&apos;s new in {status.version}</p>
                    <div className="text-xs [&_h3]:text-[11px] [&_li]:text-xs [&_p]:text-xs">
                        <AnswerMarkdown markdown={notes} />
                    </div>
                </SettingsNote>
            )}
        </div>
    );
}

export function About() {
    const [currentVersion, setCurrentVersion] = useState<string>('');

    useEffect(() => {
        // Get current version on mount
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    return (
        // specs/0066 W4: the column was left-hugging in a wide settings pane.
        <div className="mx-auto max-w-2xl space-y-4">
            {/* Header */}
            <div className="text-center">
                <h1 className="u-section-label text-[28px] tracking-[0.22em] text-foreground">NIXON</h1>
                <span aria-hidden="true" className="mt-2.5 block h-px w-full bg-border" />
                {currentVersion && (
                    <span className="text-sm text-muted-foreground"> v{currentVersion}</span>
                )}
                <p className="text-medium text-muted-foreground mt-1">
                    A local-first meeting assistant. Real-time notes and summaries that
                    never leave your machine.
                </p>
            </div>

            <UpdatesBlock />

            {/* Features Grid */}
            <div className="space-y-3">
                <div className="grid grid-cols-2 gap-2">
                    <div className="bg-muted rounded p-3">
                        <h3 className="font-bold text-sm text-foreground mb-1">Privacy-first</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Your audio, transcripts, and notes stay on your machine. No cloud, no leaks.</p>
                    </div>
                    <div className="bg-muted rounded p-3">
                        <h3 className="font-bold text-sm text-foreground mb-1">Use Any Model</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Run a local open-source model, or plug in an external API. No lock-in.</p>
                    </div>
                    <div className="bg-muted rounded p-3">
                        <h3 className="font-bold text-sm text-foreground mb-1">Cost-Smart</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Avoid pay-per-minute bills by running models locally.</p>
                    </div>
                    <div className="bg-muted rounded p-3">
                        <h3 className="font-bold text-sm text-foreground mb-1">Works everywhere</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Google Meet, Zoom, Teams — online or offline.</p>
                    </div>
                </div>
            </div>

            {/* Footer - attribution (MIT requires it) */}
            <div className="pt-2 border-t border-border text-center">
                <p className="text-xs text-muted-foreground">
                    Forked from meetily (MIT).
                </p>
            </div>
        </div>
    )
}
