import { describe, expect, it } from 'vitest';
import { tildePath } from '../format-path';

describe('tildePath', () => {
  it('abbreviates a macOS home path', () => {
    expect(tildePath('/Users/ada/Movies/nixon-recordings')).toBe('~/Movies/nixon-recordings');
  });

  it('abbreviates a Linux home path', () => {
    expect(tildePath('/home/ada/Videos/nixon-recordings')).toBe('~/Videos/nixon-recordings');
  });

  it('abbreviates the home directory itself, with no trailing slash left behind', () => {
    expect(tildePath('/Users/ada')).toBe('~');
  });

  it('keeps the rest of the path byte-for-byte, including spaces and unicode', () => {
    expect(tildePath('/Users/ada/Movies/réunions 2026/nixon-recordings')).toBe(
      '~/Movies/réunions 2026/nixon-recordings',
    );
  });

  // The whole point is removing the account name; a path that merely CONTAINS a username-ish
  // segment somewhere else must not be rewritten.
  it('only strips the prefix, never a later occurrence', () => {
    expect(tildePath('/Volumes/Backup/Users/ada/Movies')).toBe('/Volumes/Backup/Users/ada/Movies');
  });

  it('leaves a non-home absolute path alone', () => {
    expect(tildePath('/Volumes/Audio/nixon-recordings')).toBe('/Volumes/Audio/nixon-recordings');
  });

  it('does not treat /Users itself as a home directory', () => {
    expect(tildePath('/Users')).toBe('/Users');
  });

  it('passes an empty string straight through', () => {
    expect(tildePath('')).toBe('');
  });
});
