import { render } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';

// specs/0060 — the screenshot driver sets html[data-shot="1"] and needs a stable hook to
// hide dev-only chrome; this proves DevBadge carries it regardless of the dev-flags fetch.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no')) }));

import DevBadge from '../DevBadge';

describe('DevBadge shot attribute (specs/0060)', () => {
  it('carries data-dev-badge so screenshots can hide it', () => {
    const { container } = render(<DevBadge />);
    expect(container.querySelector('[data-dev-badge]')).not.toBeNull();
  });

  it('carries data-dev-badge on the collapsed variant too', () => {
    const { container } = render(<DevBadge isCollapsed />);
    expect(container.querySelector('[data-dev-badge]')).not.toBeNull();
  });
});
