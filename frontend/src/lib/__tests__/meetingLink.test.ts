import { describe, it, expect } from 'vitest';
import { parseMeetingLink } from '../meetingLink';

describe('parseMeetingLink', () => {
  it('parses a meeting link with a time', () => {
    expect(parseMeetingLink('nixon://meeting/abc?t=754.2')).toEqual({ meetingId: 'abc', t: 754.2 });
  });
  it('accepts the dev scheme, time optional', () => {
    expect(parseMeetingLink('nixon-dev://meeting/abc')).toEqual({ meetingId: 'abc', t: null });
  });
  it('decodes the id', () => {
    expect(parseMeetingLink('nixon://meeting/a%20b')?.meetingId).toBe('a b');
  });
  it.each(['?t=-3', '?t=NaN', '?t=', '?t=abc', '?t=Infinity'])('drops a bad time %s', (q) => {
    expect(parseMeetingLink(`nixon://meeting/abc${q}`)).toEqual({ meetingId: 'abc', t: null });
  });
  it.each([
    'nixon://meeting/',
    'nixon://meeting',
    'nixon://meeting/a/b',
    'nixon://record',
    'nixon://record/abc',
    'https://meeting/abc',
    'not a url',
  ])('rejects %s', (url) => {
    expect(parseMeetingLink(url)).toBeNull();
  });
});
