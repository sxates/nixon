/**
 * Lightweight HTML ⇄ Markdown helpers for the "My Notes" contentEditable editor.
 *
 * Scope is deliberately narrow. The notes toolbar only produces **bold**,
 * *italic*, `### headings`, and `- bulleted lists`, so we convert exactly those
 * (plus paragraphs and line breaks). The markdown side is the canonical store
 * the summary pipeline consumes (`notes_markdown`); the HTML side is what the
 * editor round-trips for faithful reload (`notes_json`). Pasted rich content
 * degrades best-effort rather than perfectly.
 *
 * These run client-side only (the notepad is a 'use client' component), so the
 * DOM is available for parsing.
 */

const BLOCK_TAGS = new Set(['DIV', 'P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'LI', 'BLOCKQUOTE']);

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Inline markdown for the children of an element (bold/italic/links/breaks). */
function inlineToMarkdown(node: Node): string {
  let out = '';
  node.childNodes.forEach((child) => {
    if (child.nodeType === Node.TEXT_NODE) {
      out += child.nodeValue ?? '';
      return;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) return;
    const el = child as HTMLElement;
    const tag = el.tagName;
    const inner = inlineToMarkdown(el);
    if (tag === 'B' || tag === 'STRONG') out += inner.trim() ? `**${inner}**` : inner;
    else if (tag === 'I' || tag === 'EM') out += inner.trim() ? `*${inner}*` : inner;
    else if (tag === 'BR') out += '\n';
    else if (tag === 'A') out += `[${inner}](${el.getAttribute('href') ?? ''})`;
    else out += inner;
  });
  return out;
}

function hasBlockChild(el: HTMLElement): boolean {
  return Array.from(el.children).some((c) => BLOCK_TAGS.has(c.tagName));
}

function blockToLines(node: Node, lines: string[]): void {
  if (node.nodeType === Node.TEXT_NODE) {
    const t = (node.nodeValue ?? '').replace(/\s+/g, ' ');
    if (t.trim()) lines.push(t.trim());
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node as HTMLElement;
  const tag = el.tagName;

  if (/^H[1-6]$/.test(tag)) {
    lines.push(`### ${inlineToMarkdown(el).trim()}`, '');
  } else if (tag === 'UL' || tag === 'OL') {
    let i = 1;
    Array.from(el.children).forEach((li) => {
      if (li.tagName !== 'LI') return;
      const prefix = tag === 'OL' ? `${i++}. ` : '- ';
      lines.push(prefix + inlineToMarkdown(li).trim());
    });
    lines.push('');
  } else if (tag === 'LI') {
    lines.push('- ' + inlineToMarkdown(el).trim());
  } else if (tag === 'BR') {
    lines.push('');
  } else if (tag === 'DIV' || tag === 'P' || tag === 'BLOCKQUOTE') {
    if (hasBlockChild(el)) {
      el.childNodes.forEach((c) => blockToLines(c, lines));
    } else {
      const t = inlineToMarkdown(el).trim();
      lines.push(t, ''); // blank line separates paragraphs
    }
  } else {
    const t = inlineToMarkdown(el).trim();
    if (t) lines.push(t);
  }
}

/** Serialize the notes contentEditable HTML to markdown (summary pipeline input). */
export function htmlToMarkdown(html: string): string {
  if (!html || !html.trim()) return '';
  const root = document.createElement('div');
  root.innerHTML = html.replace(/\r/g, '');
  const lines: string[] = [];
  root.childNodes.forEach((n) => blockToLines(n, lines));
  return lines.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

/** Apply inline markdown (`**bold**`, `*italic*`/`_italic_`) to an escaped string. */
function inlineMarkdownToHtml(text: string): string {
  return escapeHtml(text)
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/(^|[^*])\*([^*]+)\*/g, '$1<i>$2</i>')
    .replace(/_([^_]+)_/g, '<i>$1</i>');
}

/**
 * Best-effort markdown → HTML for the contentEditable, used when reloading notes
 * that only have a markdown store (legacy meetings that previously used BlockNote,
 * whose `notes_json` isn't our HTML). Handles headings, unordered lists, and
 * paragraphs — the same shapes our editor produces.
 */
export function markdownToHtml(md: string): string {
  if (!md || !md.trim()) return '';
  const out: string[] = [];
  let listOpen = false;
  const closeList = () => {
    if (listOpen) {
      out.push('</ul>');
      listOpen = false;
    }
  };
  for (const raw of md.replace(/\r/g, '').split('\n')) {
    const line = raw.trimEnd();
    if (/^#{1,6}\s+/.test(line)) {
      closeList();
      out.push(`<h3>${inlineMarkdownToHtml(line.replace(/^#{1,6}\s+/, ''))}</h3>`);
    } else if (/^\s*[-*+]\s+/.test(line)) {
      if (!listOpen) {
        out.push('<ul>');
        listOpen = true;
      }
      out.push(`<li>${inlineMarkdownToHtml(line.replace(/^\s*[-*+]\s+/, ''))}</li>`);
    } else if (!line.trim()) {
      closeList();
    } else {
      closeList();
      out.push(`<div>${inlineMarkdownToHtml(line)}</div>`);
    }
  }
  closeList();
  return out.join('');
}

/** Tags the notes editor itself can produce (toolbar + contentEditable defaults). */
const ALLOWED_EDITOR_TAGS = new Set([
  'B', 'STRONG', 'I', 'EM', 'H1', 'H2', 'H3', 'UL', 'OL', 'LI', 'P', 'DIV', 'BR', 'A',
]);

/** Tags whose content is never text we want (removed wholesale, not unwrapped). */
const DROPPED_TAGS = new Set([
  'SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'LINK', 'META', 'TITLE', 'NOSCRIPT',
  'TEMPLATE', 'SVG', 'MATH',
]);

/** True when an anchor href is safe to keep (no javascript:/vbscript:/data: execution). */
function isSafeHref(href: string): boolean {
  const v = href.trim().toLowerCase();
  return !v.startsWith('javascript:') && !v.startsWith('vbscript:') && !v.startsWith('data:');
}

/**
 * Reduce a node's children to the editor's own vocabulary, in place:
 * - drops script-ish tags entirely and HTML comments;
 * - maps deeper headings (h4–h6) to h3 (the toolbar's heading);
 * - unwraps everything else that isn't an allowed tag (font/span/table/etc.),
 *   keeping the text content;
 * - strips ALL attributes except a safe `a[href]` — inline `style`, `class`,
 *   `font face/color/size` and friends are exactly what makes pasted content
 *   keep its foreign styling (spec 0029 WS5.2).
 */
function cleanNodeChildren(parent: Node): void {
  // Snapshot: we mutate the child list (remove/unwrap) while iterating.
  Array.from(parent.childNodes).forEach((child) => {
    if (child.nodeType === Node.COMMENT_NODE) {
      parent.removeChild(child);
      return;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) return;
    const el = child as HTMLElement;
    const tag = el.tagName;

    if (DROPPED_TAGS.has(tag)) {
      parent.removeChild(el);
      return;
    }

    // Clean the subtree first so unwrapping lifts already-clean children.
    cleanNodeChildren(el);

    if (/^H[4-6]$/.test(tag)) {
      // Deeper headings degrade to the editor's single heading level.
      const h3 = document.createElement('h3');
      while (el.firstChild) h3.appendChild(el.firstChild);
      parent.replaceChild(h3, el);
      return;
    }

    if (!ALLOWED_EDITOR_TAGS.has(tag)) {
      // Unwrap (font/span/u/table cells/…): keep the text, lose the wrapper.
      while (el.firstChild) parent.insertBefore(el.firstChild, el);
      parent.removeChild(el);
      return;
    }

    // Allowed tag: strip every attribute except a safe href on links.
    Array.from(el.attributes).forEach((attr) => {
      if (tag === 'A' && attr.name.toLowerCase() === 'href' && isSafeHref(attr.value)) return;
      el.removeAttribute(attr.name);
    });
  });
}

/**
 * Sanitize arbitrary HTML down to the notes editor's own structure. Shared by
 * the paste handler (NoteEditor) and the load path (`sanitizeNotesHtml`): only
 * tags the toolbar can produce survive, and no attributes except `a[href]`, so
 * foreign fonts/colors/classes are discarded and scripts can't execute.
 */
export function sanitizeEditorHtml(html: string): string {
  if (!html) return '';
  const root = document.createElement('div');
  root.innerHTML = html;
  cleanNodeChildren(root);
  return root.innerHTML;
}

/**
 * Sanitize HTML loaded from storage before it's set as innerHTML. Same strict
 * pass as the paste handler — this also cleans up notes already polluted by
 * pre-0029 pastes (inline `style`/`font` survived the old sanitizer), so they
 * render in Nixon's styling on next load.
 */
export function sanitizeNotesHtml(html: string): string {
  return sanitizeEditorHtml(html);
}

/** True when a stored `notes_json` string is our editor HTML (vs legacy BlockNote JSON). */
export function looksLikeHtml(value: string | null | undefined): boolean {
  if (!value) return false;
  const t = value.trim();
  return t.startsWith('<');
}
