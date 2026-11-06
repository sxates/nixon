import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  effectiveView,
  meetingsRedirectTarget,
  readStoredView,
  storeView,
  viewFromParam,
  MEETINGS_VIEW_KEY,
  LEGACY_MEETINGS_VIEW_KEY,
  LEGACY_TODAY_VIEW_KEY,
} from '@/lib/meetings-view';

beforeEach(() => {
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe('readStoredView: one key, migrated once from the two old pages', () => {
  it('is null on a fresh profile (the page then opens on Agenda)', () => {
    expect(readStoredView()).toBeNull();
  });

  it("migrates Today's agenda ('day') and week", () => {
    window.localStorage.setItem(LEGACY_TODAY_VIEW_KEY, 'day');
    expect(readStoredView()).toBe('agenda');
    window.localStorage.clear();
    window.localStorage.setItem(LEGACY_TODAY_VIEW_KEY, 'week');
    expect(readStoredView()).toBe('week');
  });

  it("migrates All meetings' Month / List", () => {
    window.localStorage.setItem(LEGACY_MEETINGS_VIEW_KEY, 'month');
    expect(readStoredView()).toBe('month');
  });

  it("lets Today's retired day list defer to All meetings' choice", () => {
    window.localStorage.setItem(LEGACY_TODAY_VIEW_KEY, 'list');
    window.localStorage.setItem(LEGACY_MEETINGS_VIEW_KEY, 'month');
    expect(readStoredView()).toBe('month');
  });

  it('writes the new key and removes both old ones, so it runs only once', () => {
    window.localStorage.setItem(LEGACY_TODAY_VIEW_KEY, 'week');
    window.localStorage.setItem(LEGACY_MEETINGS_VIEW_KEY, 'month');
    readStoredView();
    expect(window.localStorage.getItem(MEETINGS_VIEW_KEY)).toBe('week');
    expect(window.localStorage.getItem(LEGACY_TODAY_VIEW_KEY)).toBeNull();
    expect(window.localStorage.getItem(LEGACY_MEETINGS_VIEW_KEY)).toBeNull();
  });

  it('prefers the new key over any leftover old one', () => {
    storeView('list');
    window.localStorage.setItem(LEGACY_TODAY_VIEW_KEY, 'week');
    expect(readStoredView()).toBe('list');
  });

  it('survives storage that throws (private windows)', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    expect(readStoredView()).toBeNull();
    expect(() => storeView('month')).not.toThrow();
  });
});

describe('viewFromParam: old deep links keep working', () => {
  it("reads Today's ?view=day as Agenda", () => {
    expect(viewFromParam('day')).toBe('agenda');
  });
  it('reads the four views as themselves', () => {
    for (const v of ['agenda', 'week', 'month', 'list']) expect(viewFromParam(v)).toBe(v);
  });
  it('ignores anything else', () => {
    expect(viewFromParam('grid')).toBeNull();
    expect(viewFromParam(null)).toBeNull();
  });
});

describe('effectiveView: no calendar, no Agenda/Week', () => {
  it('falls back to the List when the calendar is not connected', () => {
    expect(effectiveView('agenda', false)).toBe('list');
    expect(effectiveView('week', false)).toBe('list');
  });
  it('keeps Month and List without a calendar', () => {
    expect(effectiveView('month', false)).toBe('month');
  });
  it('keeps the choice while connected or still checking', () => {
    expect(effectiveView('week', true)).toBe('week');
    expect(effectiveView('agenda', null)).toBe('agenda');
  });
});

describe('meetingsRedirectTarget (/meetings → /)', () => {
  it('opens the List by default', () => {
    expect(meetingsRedirectTarget('')).toBe('/?view=list');
  });
  it('keeps an explicit view and the rest of the query', () => {
    expect(meetingsRedirectTarget('?view=month&processing=1')).toBe('/?view=month&processing=1');
    expect(meetingsRedirectTarget('?processing=1')).toBe('/?processing=1&view=list');
  });
});
