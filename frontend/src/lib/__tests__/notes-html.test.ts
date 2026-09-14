import { describe, it, expect } from 'vitest';
import { sanitizeEditorHtml, sanitizeNotesHtml, looksLikeHtml } from '@/lib/notes-html';

// specs/0029 WS5.2 — pasted rich content (Word/browser/Notes) kept its foreign fonts and
// styling because the old sanitizer preserved style/class/font attributes and only ran on
// load. sanitizeEditorHtml is shared by the NoteEditor paste handler and the load path
// (sanitizeNotesHtml): it reduces arbitrary HTML to the editor's own vocabulary.
describe('sanitizeEditorHtml', () => {
  it('strips inline style, class, and font-tag styling', () => {
    const dirty =
      '<div style="font-family: \'Times New Roman\'; color: red" class="MsoNormal">' +
      '<font face="Times" color="#ff0000" size="4">pasted text</font></div>';
    expect(sanitizeEditorHtml(dirty)).toBe('<div>pasted text</div>');
  });

  it('keeps the tags the toolbar can produce', () => {
    const clean =
      '<h3>Agenda</h3><div><b>bold</b> and <i>italic</i></div>' +
      '<ul><li>one</li><li>two</li></ul><ol><li>first</li></ol><p>para</p><div>a<br>b</div>';
    expect(sanitizeEditorHtml(clean)).toBe(clean);
  });

  it('keeps strong/em (browser-produced equivalents of b/i)', () => {
    expect(sanitizeEditorHtml('<p><strong>s</strong> <em>e</em></p>')).toBe(
      '<p><strong>s</strong> <em>e</em></p>',
    );
  });

  it('unwraps disallowed wrappers (span/u/table) but keeps their text', () => {
    expect(sanitizeEditorHtml('<span style="font-size:18px">hello</span>')).toBe('hello');
    expect(sanitizeEditorHtml('<u>underlined</u>')).toBe('underlined');
    expect(sanitizeEditorHtml('<table><tbody><tr><td>cell</td></tr></tbody></table>')).toBe('cell');
  });

  it('maps deeper headings (h4–h6) to the editor heading h3', () => {
    expect(sanitizeEditorHtml('<h4>four</h4><h6>six</h6>')).toBe('<h3>four</h3><h3>six</h3>');
  });

  it('preserves a[href] but drops every other attribute on links', () => {
    expect(
      sanitizeEditorHtml('<a href="https://example.com" target="_blank" class="x" style="color:blue">link</a>'),
    ).toBe('<a href="https://example.com">link</a>');
  });

  it('drops javascript: hrefs', () => {
    expect(sanitizeEditorHtml('<a href="javascript:alert(1)">click</a>')).toBe('<a>click</a>');
  });

  it('still removes scripts, styles, and inline event handlers', () => {
    const dirty =
      '<script>evil()</script><style>.x{color:red}</style>' +
      '<div onclick="evil()" onmouseover="evil()">safe text</div><iframe src="https://x"></iframe>';
    expect(sanitizeEditorHtml(dirty)).toBe('<div>safe text</div>');
  });

  it('removes HTML comments (Word pastes are full of them)', () => {
    expect(sanitizeEditorHtml('<!--[if !supportLists]--><p>item</p><!--[endif]-->')).toBe('<p>item</p>');
  });

  it('handles nested pollution recursively', () => {
    const dirty =
      '<div style="margin:0"><span class="a"><font face="Times"><b style="color:red">deep</b></font></span></div>';
    expect(sanitizeEditorHtml(dirty)).toBe('<div><b>deep</b></div>');
  });

  it('returns empty string for empty input', () => {
    expect(sanitizeEditorHtml('')).toBe('');
  });
});

describe('sanitizeNotesHtml', () => {
  it('applies the same strict pass on load, cleaning already-polluted stored notes', () => {
    const stored = '<div style="font-family:Times"><font color="red">old note</font></div>';
    expect(sanitizeNotesHtml(stored)).toBe('<div>old note</div>');
  });
});

describe('looksLikeHtml', () => {
  it('distinguishes editor HTML from legacy BlockNote JSON', () => {
    expect(looksLikeHtml('<div>x</div>')).toBe(true);
    expect(looksLikeHtml('[{"type":"paragraph"}]')).toBe(false);
    expect(looksLikeHtml(null)).toBe(false);
  });
});
