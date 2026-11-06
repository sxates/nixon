import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';

// `/meetings` (the old All meetings page) now redirects to the Meetings page at `/`,
// keeping its query and defaulting to the List view it used to open on.

const { replace } = vi.hoisted(() => ({ replace: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn(), replace }) }));

import MeetingsRedirect from '@/app/meetings/page';

beforeEach(() => {
  replace.mockReset();
});

describe('/meetings redirect', () => {
  it('replaces itself with the List view at /', () => {
    window.history.replaceState(null, '', '/meetings');
    render(<MeetingsRedirect />);
    expect(replace).toHaveBeenCalledWith('/?view=list');
  });

  it('keeps an explicit view and the rest of the query', () => {
    window.history.replaceState(null, '', '/meetings?view=month&processing=1');
    render(<MeetingsRedirect />);
    expect(replace).toHaveBeenCalledWith('/?view=month&processing=1');
  });
});
