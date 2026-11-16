import { describe, it, expect, vi, beforeAll, beforeEach } from 'vitest';
import { renderHook } from '@testing-library/react';

const push = vi.fn();
const back = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push, back }) }));

import { useBack } from '@/hooks/useBack';
import { installNavHistory, resetNavHistoryForTests } from '@/lib/nav-history';

describe('useBack', () => {
  beforeAll(() => installNavHistory());
  beforeEach(() => {
    resetNavHistoryForTests();
    push.mockReset();
    back.mockReset();
  });

  it('goes to the fallback when no in-app page is behind this one (cold deep link)', () => {
    const { result } = renderHook(() => useBack('/tasks'));
    result.current();
    expect(push).toHaveBeenCalledWith('/tasks');
    expect(back).not.toHaveBeenCalled();
  });

  it('Topic → to-do → Back returns to the topic page, not the fallback', () => {
    // Opened on the topic, then a to-do row pushed the item page.
    window.history.replaceState(null, '', '/topics/topic?id=t1');
    window.history.pushState(null, '', '/tasks/item?id=a');
    const { result } = renderHook(() => useBack('/tasks'));
    result.current();
    expect(back).toHaveBeenCalledTimes(1);
    expect(push).not.toHaveBeenCalled();
  });
});
