import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ReelLabel } from '@/components/MeetingDetails/ReelLabel';

const base = {
  date: 'Tue Jun 24',
  time: '10:00',
  length: '00:42:18',
  source: 'Zoom',
};

describe('ReelLabel', () => {
  it('prints the zero-padded reel handle in the header, beside the product mark', () => {
    render(<ReelLabel reelNumber={412} {...base} />);
    const header = screen.getByTestId('reel-label-header');
    expect(header.textContent).toBe('REEL 0412 · NIXON');
  });

  it('pads short reel numbers to four digits', () => {
    render(<ReelLabel reelNumber={7} {...base} />);
    expect(screen.getByTestId('reel-label-header').textContent).toContain('REEL 0007');
  });

  it('falls back to a blank reel handle when the number is unknown', () => {
    render(<ReelLabel reelNumber={null} {...base} />);
    expect(screen.getByTestId('reel-label-header').textContent).toContain('REEL ——');
  });

  it('renders a labelled row per field, with deck and voices when given', () => {
    render(<ReelLabel reelNumber={412} {...base} voices={4} deck="A" />);
    for (const [label, value] of [
      ['DATE', 'Tue Jun 24'],
      ['TIME', '10:00'],
      ['LENGTH', '00:42:18'],
      ['SOURCE', 'Zoom'],
      ['VOICES', '4'],
      ['DECK', 'A'],
    ]) {
      const row = screen.getByTestId(`reel-row-${label}`);
      expect(row.textContent).toContain(label);
      expect(row.textContent).toContain(value);
    }
  });

  it('omits optional rows that were not supplied', () => {
    render(<ReelLabel reelNumber={412} {...base} />);
    expect(screen.queryByTestId('reel-row-VOICES')).toBeNull();
    expect(screen.queryByTestId('reel-row-DECK')).toBeNull();
  });

  it('renders each status tag', () => {
    render(<ReelLabel reelNumber={412} {...base} tags={['Summarized', '3 tasks']} />);
    expect(screen.getByText('Summarized')).toBeInTheDocument();
    expect(screen.getByText('3 tasks')).toBeInTheDocument();
  });

  it('draws the red rule under the header as decoration only', () => {
    const { container } = render(<ReelLabel reelNumber={412} {...base} />);
    const rule = container.querySelector('[aria-hidden="true"].bg-record\\/55');
    expect(rule).not.toBeNull();
  });
});
