'use client';

/**
 * AnswerMarkdown (specs/0035) — renders an Ask-AI answer's markdown with each
 * `[M#]` citation marker turned into a link chip to its source meeting.
 *
 * Why not reuse the app's existing markdown surface: summaries render through
 * BlockNote (a full editor instance) — the wrong tool for a read-only answer
 * that needs custom inline citation chips. Instead this renders the small
 * markdown vocabulary the reduce prompt produces (headings, bold/italic,
 * bullet/numbered lists, paragraphs) directly to React nodes: plain text in,
 * elements out, no dangerouslySetInnerHTML (same posture as the command
 * palette's `renderSnippet`, specs/0033).
 *
 * Citation contract: `sources[i]` corresponds to marker `[M{i+1}]` (the Rust
 * engine's numbering). In-range markers become chips labelled with the meeting
 * title, linking to `/meeting-details?id=…`; out-of-range or malformed markers
 * render as plain text (the Rust post-processor strips these already — this is
 * defense in depth).
 */

import { useCallback, type ReactNode } from 'react';
import { useRouter } from 'next/navigation';
import type { SourceMeeting } from '@/lib/ask-ai';

const MARKER_RE = /\[M(\d+)\]/g;

/** Inline emphasis: `**bold**` then `*italic*` / `_italic_` inside the rest. */
function renderEmphasis(text: string, keyBase: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const boldParts = text.split(/\*\*([^*]+)\*\*/g);
  boldParts.forEach((part, i) => {
    if (i % 2 === 1) {
      nodes.push(<strong key={`${keyBase}-b${i}`}>{part}</strong>);
      return;
    }
    const italicParts = part.split(/(?:\*([^*\n]+)\*|_([^_\n]+)_)/g);
    // The alternation yields two capture groups per match; only one is defined.
    for (let j = 0; j < italicParts.length; j += 3) {
      const plain = italicParts[j];
      if (plain) nodes.push(plain);
      const em = italicParts[j + 1] ?? italicParts[j + 2];
      if (em !== undefined) nodes.push(<em key={`${keyBase}-i${i}-${j}`}>{em}</em>);
    }
  });
  return nodes;
}

/** Inline pass: split out `[M#]` markers, emphasize the plain segments. */
function renderInline(
  text: string,
  sources: SourceMeeting[],
  keyBase: string,
  onOpenMeeting: (meetingId: string) => void,
): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let chip = 0;
  MARKER_RE.lastIndex = 0;
  for (let m = MARKER_RE.exec(text); m !== null; m = MARKER_RE.exec(text)) {
    const num = parseInt(m[1], 10);
    const source = num >= 1 ? sources[num - 1] : undefined;
    if (!source) continue; // out-of-range → stays in the plain text below
    if (m.index > last) nodes.push(...renderEmphasis(text.slice(last, m.index), `${keyBase}-t${chip}`));
    const title = source.title?.trim() || `M${num}`;
    nodes.push(
      <button
        key={`${keyBase}-m${chip++}`}
        type="button"
        title={`Open “${title}”`}
        onClick={() => onOpenMeeting(source.meetingId)}
        className="mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 rounded-[3px] border border-brand/25 bg-brand/10 px-2 py-px align-baseline text-[11px] font-semibold leading-[1.5] text-brand transition-colors hover:bg-brand/20 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="truncate">{title}</span>
      </button>,
    );
    last = m.index + m[0].length;
  }
  if (last < text.length) nodes.push(...renderEmphasis(text.slice(last), `${keyBase}-tail`));
  return nodes;
}

interface Block {
  kind: 'heading' | 'ul' | 'ol' | 'p';
  lines: string[];
}

/** Group markdown lines into heading / list / paragraph blocks. */
function toBlocks(markdown: string): Block[] {
  const blocks: Block[] = [];
  for (const raw of markdown.replace(/\r/g, '').split('\n')) {
    const line = raw.trimEnd();
    if (!line.trim()) continue;
    if (/^#{1,6}\s+/.test(line)) {
      blocks.push({ kind: 'heading', lines: [line.replace(/^#{1,6}\s+/, '')] });
    } else if (/^\s*[-*+]\s+/.test(line)) {
      const item = line.replace(/^\s*[-*+]\s+/, '');
      const prev = blocks[blocks.length - 1];
      if (prev?.kind === 'ul') prev.lines.push(item);
      else blocks.push({ kind: 'ul', lines: [item] });
    } else if (/^\s*\d+[.)]\s+/.test(line)) {
      const item = line.replace(/^\s*\d+[.)]\s+/, '');
      const prev = blocks[blocks.length - 1];
      if (prev?.kind === 'ol') prev.lines.push(item);
      else blocks.push({ kind: 'ol', lines: [item] });
    } else {
      blocks.push({ kind: 'p', lines: [line.trim()] });
    }
  }
  return blocks;
}

export function AnswerMarkdown({
  markdown,
  sources,
}: {
  markdown: string;
  sources: SourceMeeting[];
}) {
  const router = useRouter();
  const onOpenMeeting = useCallback(
    (meetingId: string) => router.push(`/meeting-details?id=${meetingId}`),
    [router],
  );

  const blocks = toBlocks(markdown);
  return (
    <div className="text-[14.5px] leading-relaxed text-foreground">
      {blocks.map((block, i) => {
        const key = `blk-${i}`;
        switch (block.kind) {
          case 'heading':
            return (
              <h3 key={key} className="u-doc-heading mb-1.5 mt-5 first:mt-0">
                {renderInline(block.lines[0], sources, key, onOpenMeeting)}
              </h3>
            );
          case 'ul':
          case 'ol': {
            const List = block.kind === 'ul' ? 'ul' : 'ol';
            return (
              <List
                key={key}
                className={`my-2 flex flex-col gap-1 pl-5 ${block.kind === 'ul' ? 'list-disc' : 'list-decimal'}`}
              >
                {block.lines.map((item, j) => (
                  <li key={`${key}-li${j}`}>
                    {renderInline(item, sources, `${key}-li${j}`, onOpenMeeting)}
                  </li>
                ))}
              </List>
            );
          }
          case 'p':
            return (
              <p key={key} className="my-2 first:mt-0 last:mb-0">
                {renderInline(block.lines[0], sources, key, onOpenMeeting)}
              </p>
            );
        }
      })}
    </div>
  );
}
