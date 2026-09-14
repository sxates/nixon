import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

// Radix popovers/menus call Pointer Capture + scrollIntoView APIs that jsdom doesn't
// implement; without these stubs a DropdownMenu won't open under fireEvent. Stub them
// once globally so component tests can drive Radix menus (e.g. SegmentSpeakerMenu).
for (const name of [
  'hasPointerCapture',
  'setPointerCapture',
  'releasePointerCapture',
  'scrollIntoView',
] as const) {
  if (!(name in Element.prototype)) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (Element.prototype as any)[name] = () => {};
  }
}

// Node 22+ exposes its own experimental `localStorage`/`sessionStorage` globals, and on
// those runtimes they shadow jsdom's Storage with an object that has no getItem/setItem/
// clear at all (this repo targets Node 20, where jsdom's own Storage is used and the
// branch below is a no-op). Install a minimal in-memory Storage so tests that exercise
// persistence (e.g. ThemeContext) behave the same on both.
function installMemoryStorage(key: 'localStorage' | 'sessionStorage') {
  const existing = (globalThis as unknown as Record<string, unknown>)[key] as
    | { getItem?: unknown }
    | undefined;
  if (existing && typeof existing.getItem === 'function') return;
  const map = new Map<string, string>();
  const storage = {
    getItem: (k: string) => (map.has(String(k)) ? map.get(String(k))! : null),
    setItem: (k: string, v: string) => void map.set(String(k), String(v)),
    removeItem: (k: string) => void map.delete(String(k)),
    clear: () => map.clear(),
    key: (i: number) => Array.from(map.keys())[i] ?? null,
    get length() {
      return map.size;
    },
  };
  Object.defineProperty(globalThis, key, { configurable: true, value: storage });
}
installMemoryStorage('localStorage');
installMemoryStorage('sessionStorage');

// cmdk (the ⌘K command palette) observes its list with ResizeObserver, which jsdom
// doesn't implement. A no-op stub is enough — tests don't depend on resize behavior.
if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// jsdom shares one global window across a file's tests, so storage and rendered
// DOM leak between cases unless we reset them. Keep every test isolated.
afterEach(() => {
  cleanup();
  try {
    window.sessionStorage.clear();
  } catch {
    /* sessionStorage unavailable in this env */
  }
  try {
    window.localStorage.clear();
  } catch {
    /* localStorage unavailable in this env */
  }
});
