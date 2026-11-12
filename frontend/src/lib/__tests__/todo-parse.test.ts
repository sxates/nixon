import { describe, it, expect } from 'vitest';
import fixture from '../__fixtures__/due-dates.json';
import {
  parseTodoText,
  resolveDuePhrase,
  serializeMentions,
  deserializeMentions,
  titleSegments,
} from '../todo-parse';

// Local-date anchor from a YYYY-MM-DD fixture string (never via UTC).
function day(iso: string): Date {
  const [y, m, d] = iso.split('-').map(Number);
  return new Date(y, m - 1, d);
}

describe('resolveDuePhrase (shared fixture)', () => {
  for (const c of fixture.phrases) {
    it(`${JSON.stringify(c.text)} on ${c.today}`, () => {
      expect(resolveDuePhrase(c.text, day(c.today))).toBe(c.expect);
    });
  }
});

describe('parseTodoText (shared fixture)', () => {
  for (const c of fixture.titles) {
    it(JSON.stringify(c.input), () => {
      expect(parseTodoText(c.input, day(c.today))).toEqual({
        title: c.title,
        dueDate: c.due,
      });
    });
  }
});

describe('mentions', () => {
  it('serialize/deserialize round-trip', () => {
    const picked = [{ name: 'Sam Rivera', personId: 'p-1' }];
    const desc = serializeMentions('Check in with @Sam Rivera today', picked);
    expect(desc).toBe('Check in with @[Sam Rivera](person:p-1) today');
    expect(deserializeMentions(desc)).toEqual({
      text: 'Check in with @Sam Rivera today',
      picked,
    });
  });

  it('unpicked @word stays plain', () => {
    expect(serializeMentions('ping @sam later', [])).toBe('ping @sam later');
    expect(deserializeMentions('ping @sam later')).toEqual({
      text: 'ping @sam later',
      picked: [],
    });
  });

  it('strips brackets and parens from names before writing a token', () => {
    const out = serializeMentions('hi @A]b)c[d', [{ name: 'A]b)c[d', personId: 'p-2' }]);
    expect(out).toBe('hi @[Abcd](person:p-2)');
    expect(out).toMatch(/@\[([^\]]+)\]\(person:([^)]+)\)/);
  });

  it('titleSegments uses renamed person', () => {
    const desc = 'Check in with @[Sam](person:p-1) soon';
    const segs = titleSegments(desc, new Map([['p-1', 'Samantha']]));
    expect(segs).toEqual([
      { kind: 'text', text: 'Check in with ' },
      { kind: 'person', personId: 'p-1', name: 'Samantha' },
      { kind: 'text', text: ' soon' },
    ]);
  });

  it('titleSegments falls back to the token name', () => {
    const segs = titleSegments('@[Sam](person:p-9)', new Map());
    expect(segs).toEqual([{ kind: 'person', personId: 'p-9', name: 'Sam' }]);
  });
});
