/**
 * To-do quick-add parsing and person-mention tokens (specs/0088).
 *
 * `resolveDuePhrase` mirrors the Rust resolver (`src-tauri/src/action_items/due.rs`);
 * both are pinned by `__fixtures__/due-dates.json`. All dates are LOCAL calendar dates
 * written as `YYYY-MM-DD` — never go through UTC (`toISOString`).
 */

const WEEKDAYS: [string, string][] = [
  ['monday', 'mon'],
  ['tuesday', 'tue'],
  ['wednesday', 'wed'],
  ['thursday', 'thu'],
  ['friday', 'fri'],
  ['saturday', 'sat'],
  ['sunday', 'sun'],
];

const MONTHS: [string, string][] = [
  ['january', 'jan'],
  ['february', 'feb'],
  ['march', 'mar'],
  ['april', 'apr'],
  ['may', 'may'],
  ['june', 'jun'],
  ['july', 'jul'],
  ['august', 'aug'],
  ['september', 'sep'],
  ['october', 'oct'],
  ['november', 'nov'],
  ['december', 'dec'],
];

const pad = (n: number) => String(n).padStart(2, '0');

function iso(y: number, m: number, d: number): string {
  return `${y}-${pad(m)}-${pad(d)}`;
}

/** Monday = 0 … Sunday = 6. */
const mondayIndex = (d: Date) => (d.getDay() + 6) % 7;

function addDays(today: Date, n: number): string {
  const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() + n);
  return iso(d.getFullYear(), d.getMonth() + 1, d.getDate());
}

function parseDay(s: string): number | null {
  return /^\d{1,2}$/.test(s) ? Number(s) : null;
}

/** True when month/day exist in `year` (Feb 30, month 13, Feb 29 in a common year do not). */
function validDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1) return false;
  const d = new Date(year, month - 1, day);
  return d.getFullYear() === year && d.getMonth() === month - 1 && d.getDate() === day;
}

/** This year, or the next year when it has passed (or doesn't exist this year). */
function monthDay(month: number, day: number, today: Date): string | null {
  const y = today.getFullYear();
  const todayKey = iso(y, today.getMonth() + 1, today.getDate());
  if (validDate(y, month, day)) {
    const key = iso(y, month, day);
    if (key >= todayKey) return key;
  }
  return validDate(y + 1, month, day) ? iso(y + 1, month, day) : null;
}

/**
 * Resolves the text inside a due-date parenthesis (trimmed, case-insensitive). Same
 * weekday as today = today; `this <weekday>` = the bare weekday; `next <weekday>` = that
 * weekday in the next Monday-start week; `next week` = the Monday after today; M/D is US
 * order.
 */
export function resolveDuePhrase(phrase: string, today: Date): string | null {
  const words = phrase.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const norm = words.join(' ');

  if (norm === 'today') return addDays(today, 0);
  if (norm === 'tomorrow') return addDays(today, 1);
  if (norm === 'next week') return addDays(today, 7 - mondayIndex(today));

  // `this <weekday>` = bare weekday; `next <weekday>` = that weekday in the next
  // (Monday-start) calendar week.
  const qualified = /^(this|next) (\S+)$/.exec(norm);
  const which = qualified ? qualified[1] : null;
  const wdText = qualified ? qualified[2] : norm;
  const wd = WEEKDAYS.findIndex(([full, abbr]) => wdText === full || wdText === abbr);
  if (wd >= 0) {
    if (which === 'next') return addDays(today, 7 - mondayIndex(today) + wd);
    return addDays(today, (((wd - mondayIndex(today)) % 7) + 7) % 7);
  }

  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(norm);
  if (m) {
    const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
    return validDate(y, mo, d) ? norm : null;
  }

  if (words.length === 2) {
    const month = MONTHS.findIndex(([full, abbr]) => words[0] === full || words[0] === abbr);
    const day = parseDay(words[1]);
    return month >= 0 && day !== null ? monthDay(month + 1, day, today) : null;
  }

  const slash = /^(\d+)\/(\d+)$/.exec(norm);
  if (slash) {
    const month = parseDay(slash[1]);
    const day = parseDay(slash[2]);
    return month !== null && day !== null ? monthDay(month, day, today) : null;
  }

  return null;
}

const HINT_PREFIXES = ['by ', 'on ', 'due ', 'before '];

/**
 * Resolves an extraction `due` hint ("by Friday", "(Nov 20)", "2026-07-10") against
 * `anchor` (the local date the hint was said). Mirrors Rust `resolve_due_hint`.
 */
export function resolveDueHint(hint: string, anchor: Date): string | null {
  let h = hint.trim();
  if (h.startsWith('(') && h.endsWith(')')) h = h.slice(1, -1).trim();
  const lower = h.toLowerCase();
  const prefix = HINT_PREFIXES.find((p) => lower.startsWith(p));
  if (prefix) h = h.slice(prefix.length).trim();
  return resolveDuePhrase(h, anchor);
}

const TRAILING_PAREN = /\s*\*?\s*\(([^)]*)\)\s*$/;

/**
 * Splits a quick-add line into title and due date. A trailing `(phrase)` (optionally
 * preceded by `*`) is removed only when the phrase resolves; otherwise the text is kept
 * verbatim.
 */
export function parseTodoText(
  text: string,
  today: Date,
): { title: string; dueDate: string | null } {
  const m = TRAILING_PAREN.exec(text);
  if (m) {
    const due = resolveDuePhrase(m[1], today);
    if (due) return { title: text.slice(0, m.index).trim(), dueDate: due };
  }
  return { title: text, dueDate: null };
}

// ---- person mentions: stored as `@[Display Name](person:<id>)` ------------------------

export interface PickedMention {
  name: string;
  personId: string;
}

// Same shape as the Rust token regex (action_items plain_description).
const TOKEN = /@\[([^\]]+)\]\(person:([^)]+)\)/g;

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Turns each picked `@Name` into a token. Brackets/parens are stripped from the name
 *  first — a `]` or `)` would break the token grammar. Unpicked `@words` stay plain. */
export function serializeMentions(text: string, picked: PickedMention[]): string {
  let out = text;
  // Longest first so "@Sam Rivera" wins over "@Sam".
  for (const p of [...picked].sort((a, b) => b.name.length - a.name.length)) {
    const safe = p.name.replace(/[\[\]()]/g, '');
    if (!safe) continue;
    const re = new RegExp(`@${escapeRe(p.name)}(?![\\p{L}\\p{N}_])`, 'gu');
    out = out.replace(re, () => `@[${safe}](person:${p.personId})`);
  }
  return out;
}

/** Inverse of `serializeMentions`: tokens back to `@Name`, with the picked list. */
export function deserializeMentions(desc: string): {
  text: string;
  picked: PickedMention[];
} {
  const picked: PickedMention[] = [];
  const text = desc.replace(TOKEN, (_all, name: string, personId: string) => {
    if (!picked.some((p) => p.personId === personId)) picked.push({ name, personId });
    return `@${name}`;
  });
  return { text, picked };
}

export type TitleSegment =
  | { kind: 'text'; text: string }
  | { kind: 'person'; personId: string; name: string };

/** Splits a stored description into text and person chips, using each person's CURRENT
 *  name from `nameById` (falling back to the name in the token). */
export function titleSegments(desc: string, nameById: Map<string, string>): TitleSegment[] {
  const segs: TitleSegment[] = [];
  let last = 0;
  for (const m of desc.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) segs.push({ kind: 'text', text: desc.slice(last, at) });
    segs.push({ kind: 'person', personId: m[2], name: nameById.get(m[2]) ?? m[1] });
    last = at + m[0].length;
  }
  if (last < desc.length) segs.push({ kind: 'text', text: desc.slice(last) });
  return segs;
}
