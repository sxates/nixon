import { describe, it, expect } from 'vitest';
import { useState } from 'react';
import { render, screen } from '@testing-library/react';

import { NavSlotPortal, NavSlotProvider } from '@/components/Meetings/NavSlot';

function Page() {
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  return (
    <>
      <div data-testid="tab-row">
        <div ref={setSlot} data-testid="slot" />
      </div>
      <NavSlotProvider value={slot}>
        <div data-testid="content">
          <NavSlotPortal>
            <button type="button">Next day</button>
          </NavSlotPortal>
        </div>
      </NavSlotProvider>
    </>
  );
}

describe('NavSlot', () => {
  it('moves a view\'s navigation into the tab row when the slot is mounted', () => {
    render(<Page />);
    const button = screen.getByRole('button', { name: 'Next day' });
    expect(screen.getByTestId('slot')).toContainElement(button);
    expect(screen.getByTestId('content')).not.toContainElement(button);
  });

  it('renders in place when there is no slot', () => {
    render(
      <div data-testid="content">
        <NavSlotPortal>
          <button type="button">Next day</button>
        </NavSlotPortal>
      </div>,
    );
    expect(screen.getByTestId('content')).toContainElement(
      screen.getByRole('button', { name: 'Next day' }),
    );
  });
});
