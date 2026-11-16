import { describe, it, expect, beforeAll, beforeEach } from 'vitest';
import { canGoBackInApp, installNavHistory, resetNavHistoryForTests } from '@/lib/nav-history';

const pop = (state: unknown) => window.dispatchEvent(new PopStateEvent('popstate', { state }));

describe('nav-history', () => {
  beforeAll(() => installNavHistory());
  beforeEach(() => resetNavHistoryForTests());

  it('the page the app opened on has nothing in-app behind it', () => {
    expect(canGoBackInApp()).toBe(false);
  });

  it('a push puts an in-app page behind the new one, and stamps the entry', () => {
    window.history.pushState({ __NA: true }, '', '/topics/topic?id=t1');
    expect(canGoBackInApp()).toBe(true);
    expect(window.history.state).toMatchObject({ __NA: true, __nixonNavIdx: 1 });
  });

  it('a replace keeps the position (a URL filter change is not a page to go back to)', () => {
    window.history.replaceState(null, '', '/tasks?tab=todo');
    expect(canGoBackInApp()).toBe(false);
    window.history.pushState(null, '', '/tasks/item?id=a');
    window.history.replaceState(null, '', '/tasks/item?id=a&x=1');
    expect(window.history.state).toMatchObject({ __nixonNavIdx: 1 });
  });

  it('popstate takes the position stamped on the entry it lands on', () => {
    window.history.pushState(null, '', '/topics/topic?id=t1');
    window.history.pushState(null, '', '/tasks/item?id=a');
    pop({ __nixonNavIdx: 1 }); // back to the topic
    expect(canGoBackInApp()).toBe(true);
    pop({ __nixonNavIdx: 0 }); // back to the first page
    expect(canGoBackInApp()).toBe(false);
    pop({ __nixonNavIdx: 2 }); // forward again
    expect(canGoBackInApp()).toBe(true);
  });

  it('an entry from before the session (no stamp) has nothing in-app behind it', () => {
    window.history.pushState(null, '', '/people');
    pop(null);
    expect(canGoBackInApp()).toBe(false);
  });

  it('installing twice does not double-count pushes', () => {
    installNavHistory();
    window.history.pushState(null, '', '/a');
    expect(window.history.state).toMatchObject({ __nixonNavIdx: 1 });
  });
});
