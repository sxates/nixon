'use client';

import { useEffect, useRef, useState } from "react";

/**
 * specs/0061 W5 (task 5) — inline correction for one transcript line's text (the
 * first external user's "the anodised and closure lead time" garble). A transcript
 * row swaps its rendered paragraph for this editor in place.
 *
 * Keys: Enter saves the trimmed text; Shift+Enter inserts a newline (the textarea's
 * own default — no handling needed here); Escape discards the edit; losing focus
 * (blur) also saves, matching the other inline editors in this app. Escape and blur
 * must not fight each other — cancelling, then losing focus as the editor is torn
 * down, must not ALSO fire a save of the discarded text.
 *
 * A no-op edit (trimmed text unchanged from `initialText`) is treated as a cancel,
 * not a save (specs/0061 review, I2): clicking the pencil and clicking away without
 * changing anything is the natural "changed my mind" gesture, and a real save marks
 * the line `edited` and discards its per-word timestamps server-side — a cost this
 * gesture must not pay.
 */
export interface SegmentTextEditorProps {
    /** The RAW segment text — never the stop-word-cleaned displayText. Editing must
     *  not silently persist the cleaned form. */
    initialText: string;
    /** Resolves true on a successful save, false on failure. On failure the caller
     *  (the transcript row) keeps this editor mounted — with the text the user
     *  typed still in it — rather than discarding it (specs/0061 W5 review, R40);
     *  this editor un-guards itself so the next Enter/blur can retry the same save. */
    onSave: (text: string) => Promise<boolean>;
    onCancel: () => void;
}

export function SegmentTextEditor({ initialText, onSave, onCancel }: SegmentTextEditorProps) {
    const [value, setValue] = useState(initialText);
    const textareaRef = useRef<HTMLTextAreaElement>(null);
    // Enter-save and Escape-cancel both end the edit, and the field then typically
    // loses focus (removed/unmounted by the parent). This guards the blur handler
    // below from firing a SECOND save — or a save after a cancel — for that same
    // focus loss.
    const settledRef = useRef(false);

    useEffect(() => {
        const el = textareaRef.current;
        el?.focus();
        el?.select();
    }, []);

    const commitSave = () => {
        if (settledRef.current) return;
        settledRef.current = true;
        const trimmed = value.trim();
        if (trimmed === initialText.trim()) {
            // No-op edit: treat exactly like a cancel — nothing changed, so nothing
            // is saved, and the line's `edited` mark / word timestamps stay intact.
            onCancel();
            return;
        }
        void onSave(trimmed).then((ok) => {
            // A successful save unmounts this editor (nothing left to guard). A
            // failed one keeps it mounted with the typed text still in it — un-guard
            // so the next Enter/blur can retry the same save.
            if (!ok) settledRef.current = false;
        });
    };

    const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            commitSave();
        } else if (e.key === 'Escape') {
            e.preventDefault();
            settledRef.current = true;
            onCancel();
        }
        // Shift+Enter: no handling — the textarea's default newline insertion applies.
    };

    return (
        <textarea
            ref={textareaRef}
            aria-label="Edit transcript line"
            className="u-typed w-full resize-none rounded-[3px] border border-input bg-transparent px-2 py-1 focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={handleKeyDown}
            onBlur={commitSave}
            rows={2}
        />
    );
}
