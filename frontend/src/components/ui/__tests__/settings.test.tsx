import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import {
  SettingsGroup,
  SettingsNote,
  SettingsRow,
  SettingsSection,
} from '../settings';

describe('SettingsRow', () => {
  it('renders the label, description and control', () => {
    render(
      <SettingsGroup>
        <SettingsRow
          label="Transcribe in real time"
          description="Show a live transcript while you record."
          control={<button type="button">Toggle</button>}
        />
      </SettingsGroup>,
    );

    expect(screen.getByText('Transcribe in real time')).toBeInTheDocument();
    expect(
      screen.getByText('Show a live transcript while you record.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Toggle' })).toBeInTheDocument();
  });

  it('renders a real <label> associated with the control when htmlFor is given', () => {
    render(
      <SettingsRow
        label="Expected number of speakers"
        htmlFor="speaker-count"
        control={<input id="speaker-count" type="number" />}
      />,
    );

    const label = screen.getByText('Expected number of speakers');
    expect(label.tagName).toBe('LABEL');
    expect(label).toHaveAttribute('for', 'speaker-count');
    // The association is what makes the control accessible by its label.
    expect(
      screen.getByLabelText('Expected number of speakers'),
    ).toHaveAttribute('id', 'speaker-count');
  });

  it('renders a plain element (not a label) without htmlFor', () => {
    render(<SettingsRow label="Speaker diarization" />);
    expect(screen.getByText('Speaker diarization').tagName).not.toBe('LABEL');
  });

  it('renders children full-width below the label', () => {
    render(
      <SettingsRow label="Appearance">
        <div data-testid="wide-control">radiogroup</div>
      </SettingsRow>,
    );
    const wide = screen.getByTestId('wide-control');
    // Not inside the label/control flex line — it is a sibling block beneath it.
    expect(wide.parentElement).not.toHaveClass('flex-none');
    expect(screen.getByText('Appearance')).toBeInTheDocument();
  });

  it('aligns to the top when align="start"', () => {
    const { container } = render(
      <SettingsRow label="Tall control" align="start" control={<span>x</span>} />,
    );
    const line = container.querySelector('.justify-between') as HTMLElement;
    expect(line).toHaveClass('items-start');
    expect(line).not.toHaveClass('items-center');
  });
});

describe('SettingsNote', () => {
  it('applies the info tone classes', () => {
    render(<SettingsNote tone="info">Heads up</SettingsNote>);
    const note = screen.getByText('Heads up');
    expect(note).toHaveClass('bg-brand/10');
    expect(note).toHaveClass('border-brand/30');
    expect(note).toHaveClass('rounded-[3px]');
  });

  it('applies the warn tone classes', () => {
    render(<SettingsNote tone="warn">Careful</SettingsNote>);
    const note = screen.getByText('Careful');
    expect(note).toHaveClass('bg-record/10');
    expect(note).toHaveClass('border-record/30');
  });

  it('defaults to the muted tone', () => {
    render(<SettingsNote>Just so you know</SettingsNote>);
    const note = screen.getByText('Just so you know');
    expect(note).toHaveClass('bg-muted');
    expect(note).toHaveAttribute('data-tone', 'muted');
  });
});

describe('SettingsSection', () => {
  it('labels the region via aria-labelledby pointing at the heading', () => {
    render(
      <SettingsSection title="Recording" description="How meetings are captured.">
        <div>body</div>
      </SettingsSection>,
    );

    const region = screen.getByRole('region', { name: 'Recording' });
    const heading = screen.getByRole('heading', { name: 'Recording' });
    expect(heading).toHaveClass('u-section-label');
    expect(region).toHaveAttribute('aria-labelledby', heading.id);
    expect(heading.id).toBeTruthy();
    expect(screen.getByText('How meetings are captured.')).toHaveClass('u-meta');
  });

  it('uses an explicit id when one is given', () => {
    render(<SettingsSection id="appearance-label" title="Appearance" />);
    expect(screen.getByRole('heading', { name: 'Appearance' })).toHaveAttribute(
      'id',
      'appearance-label',
    );
    expect(screen.getByRole('region', { name: 'Appearance' })).toHaveAttribute(
      'aria-labelledby',
      'appearance-label',
    );
  });
});
