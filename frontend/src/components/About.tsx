import React, { useState, useEffect } from "react";
import { getVersion } from '@tauri-apps/api/app';

export function About() {
    const [currentVersion, setCurrentVersion] = useState<string>('');

    useEffect(() => {
        // Get current version on mount
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    return (
        <div className="max-w-2xl space-y-4">
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
