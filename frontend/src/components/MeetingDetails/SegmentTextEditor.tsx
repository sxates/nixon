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
 */
export interface SegmentTextEditorProps {
    /** The RAW segment text — never the stop-word-cleaned displayText. Editing must
     *  not silently persist the cleaned form. */
    initialText: string;
    /** Resolves true on a successful save, false on failure. The caller (the
     *  transcript row/view) owns the optimistic update and its revert; this editor
     *  doesn't need to know which happened. */
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
        void onSave(value.trim());
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
