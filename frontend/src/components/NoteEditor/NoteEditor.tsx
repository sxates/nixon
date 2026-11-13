'use client';

import { useCallback, useEffect, useRef } from 'react';

import { Button } from '@/components/ui/button';
import { sanitizeEditorHtml } from '@/lib/notes-html';

/**
 * Lightweight notes editor — a plain `contentEditable` div with a minimal
 * Bold / Italic / Heading / Bullet toolbar, matching the redesign mockup
 * (design source: specs/0057 mockup). Deliberately NOT BlockNote: no slash
 * menu, block selector, or drag handles. Content round-trips as HTML; the
 * parent converts to markdown for the summary pipeline (see lib/notes-html).
 *
 * Uncontrolled: `initialHtml` seeds the editor once. To load different content
 * (e.g. switching meetings) remount via `key`. `onChange` fires the current HTML
 * on every edit, including toolbar formatting.
 */
interface NoteEditorProps {
  initialHtml: string;
  onChange: (html: string) => void;
  editable?: boolean;
  placeholder?: string;
  /** Small uppercase section label shown left of the toolbar (record screen). */
  label?: string;
  /** Extra classes on the outer wrapper. */
  className?: string;
  /** Extra classes on the scrollable editable body. */
  bodyClassName?: string;
  /**
   * Lay the editor out in the caller's own frame instead of the built-in label row: it gets
   * the formatting toolbar and the editable body as nodes (the Prep tab puts them in a
   * `Panel` — title on the panel, toolbar in its `action` slot). `label` is ignored then.
   */
  frame?: (parts: { toolbar: React.ReactNode; body: React.ReactNode }) => React.ReactNode;
  /** Imperative hooks for the owning panel (quick-add appends a "☐ To-do:" line). */
  apiRef?: React.MutableRefObject<NoteEditorApi | null>;
  /** Accessible name of the editable body. */
  ariaLabel?: string;
}

export interface NoteEditorApi {
  /** Add `text` as a new last paragraph and report the change through `onChange`. */
  appendParagraph: (text: string) => void;
}

export function NoteEditor({
  initialHtml,
  onChange,
  editable = true,
  placeholder = "Type your notes… they'll be woven into the summary.",
  label,
  className = '',
  bodyClassName = '',
  frame,
  apiRef,
  ariaLabel = 'Meeting notes',
}: NoteEditorProps) {
  const elRef = useRef<HTMLDivElement | null>(null);

  // Seed innerHTML exactly once (uncontrolled), so caret/selection aren't reset on re-render.
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      elRef.current = node;
      if (node && node.dataset.init !== '1') {
        node.innerHTML = initialHtml || '';
        node.dataset.init = '1';
      }
    },
    [initialHtml],
  );

  const emit = useCallback(() => {
    if (elRef.current) onChange(elRef.current.innerHTML);
  }, [onChange]);

  useEffect(() => {
    if (!apiRef) return;
    apiRef.current = {
      appendParagraph: (text: string) => {
        const el = elRef.current;
        if (!el) return;
        const p = document.createElement('p');
        p.textContent = text;
        el.appendChild(p);
        emit();
      },
    };
    return () => {
      apiRef.current = null;
    };
  }, [apiRef, emit]);

  // Toolbar commands run on the live selection; preventDefault keeps focus/selection
  // in the editor. execCommand also fires 'input', but we emit explicitly to be safe.
  const cmd = (command: string, value?: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    elRef.current?.focus();
    try {
      document.execCommand(command, false, value);
    } catch {
      /* best-effort rich text */
    }
    emit();
  };

  // Intercept paste so clipboard HTML (Word/browser/Notes) is reduced to the editor's own
  // vocabulary — without this the browser inserts the full clipboard markup, inline
  // font-family and all (spec 0029 WS5.2). execCommand keeps undo history and caret
  // placement consistent with the toolbar's existing usage.
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLDivElement>) => {
      e.preventDefault();
      const html = e.clipboardData.getData('text/html');
      const text = e.clipboardData.getData('text/plain');
      try {
        const clean = html ? sanitizeEditorHtml(html) : '';
        if (clean.trim()) {
          document.execCommand('insertHTML', false, clean);
        } else if (text) {
          document.execCommand('insertText', false, text);
        }
      } catch {
        /* best-effort rich text */
      }
      emit();
    },
    [emit],
  );

  const showToolbar = editable;

  const toolbar = showToolbar ? (
    <div className="flex items-center gap-0.5">
      <ToolbarButton title="Bold" onMouseDown={cmd('bold')}>
        <span className="text-meta font-bold">B</span>
      </ToolbarButton>
      <ToolbarButton title="Italic" onMouseDown={cmd('italic')}>
        <span className="font-display text-meta italic">i</span>
      </ToolbarButton>
      <ToolbarButton title="Heading" onMouseDown={cmd('formatBlock', 'h3')}>
        <span className="text-xs font-bold">H</span>
      </ToolbarButton>
      <ToolbarButton title="Bulleted list" onMouseDown={cmd('insertUnorderedList')}>
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden>
          <circle cx="2.5" cy="3.5" r="1.3" fill="currentColor" />
          <circle cx="2.5" cy="10.5" r="1.3" fill="currentColor" />
          <line x1="6" y1="3.5" x2="12" y2="3.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          <line x1="6" y1="10.5" x2="12" y2="10.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </ToolbarButton>
    </div>
  ) : null;

  const body = (
    <div
      ref={refCb}
      contentEditable={editable}
      suppressContentEditableWarning
      data-ph={placeholder}
      onInput={emit}
      onBlur={emit}
      onPaste={handlePaste}
      role="textbox"
      aria-multiline="true"
      aria-label={ariaLabel}
      className={`note-editor min-h-0 flex-1 overflow-y-auto text-sm leading-[1.65] text-foreground ${bodyClassName}`}
    />
  );

  if (frame) return <>{frame({ toolbar, body })}</>;

  return (
    <div className={`flex min-h-0 flex-col ${className}`}>
      {(label || showToolbar) && (
        <div className="flex items-center justify-between px-1 pb-2">
          {label ? (
            <span className="u-section-label">{label}</span>
          ) : (
            <span />
          )}
          {toolbar}
        </div>
      )}
      {body}
    </div>
  );
}

function ToolbarButton({
  title,
  onMouseDown,
  children,
}: {
  title: string;
  onMouseDown: (e: React.MouseEvent) => void;
  children: React.ReactNode;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      title={title}
      onMouseDown={onMouseDown}
      className="text-muted-foreground hover:text-foreground"
    >
      {children}
    </Button>
  );
}
