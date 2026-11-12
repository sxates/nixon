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
    // Callout's info tone: the same 10% / 30% brand values, spelled as the named washes.
    expect(note).toHaveClass('bg-brand-wash');
    expect(note).toHaveClass('border-brand-edge');
    // Callout's own geometry, like every callout (docs/design-system.md §Radius).
    expect(note).toHaveClass('rounded-lg', 'px-4', 'py-3');
    expect(note).not.toHaveClass('rounded-md');
    expect(note).not.toHaveClass('p-4');
  });

  it('applies the warn tone classes (amber, never the REC red)', () => {
    render(<SettingsNote tone="warn">Careful</SettingsNote>);
    const note = screen.getByText('Careful');
    expect(note).toHaveClass('bg-brand-wash');
    expect(note).toHaveClass('border-brand');
    expect(note.className).not.toMatch(/record/);
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

  it('puts an action on the title line', () => {
    render(
      <SettingsSection title="Summary prompts" action={<button type="button">New prompt</button>}>
        <div>body</div>
      </SettingsSection>,
    );
    const heading = screen.getByRole('heading', { name: 'Summary prompts' });
    const action = screen.getByRole('button', { name: 'New prompt' });
    // Same header: the heading and the action share it, the action in its own column.
    expect(heading.parentElement).toHaveAttribute('data-section-header');
    expect(heading.parentElement).toContainElement(action);
    // The 32px action must not stretch the title row: negative margins take it out of the
    // row height, so the rhythm matches a section without an action.
    expect(action.parentElement).toHaveClass('-my-2.5');
    // The region is still named by the heading alone.
    expect(screen.getByRole('region', { name: 'Summary prompts' })).toBeInTheDocument();
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

describe('SettingsGroup', () => {
  it('is a 4px card like every other card (rounded-lg)', () => {
    const { container } = render(<SettingsGroup>rows</SettingsGroup>);
    expect(container.firstElementChild).toHaveClass('rounded-lg', 'border-border', 'bg-card');
    expect(container.firstElementChild).not.toHaveClass('rounded-md');
  });
});

describe('SettingsRow aside, muted, attributes', () => {
  it('puts the aside on the label line and keeps the label at text-sm', () => {
    render(<SettingsRow label="Standard" aside={<span>Default</span>} description="Body" />);
    const label = screen.getByText('Standard');
    expect(label).toHaveClass('text-sm', 'font-medium', 'text-foreground', 'truncate');
    expect(label.parentElement).toContainElement(screen.getByText('Default'));
  });

  it('mutes the label', () => {
    render(<SettingsRow label="macOS Calendar" muted />);
    expect(screen.getByText('macOS Calendar')).toHaveClass('text-muted-foreground');
  });

  it('passes role and aria-label to the row', () => {
    render(<SettingsRow role="group" aria-label="Google Calendar source" label="Google Calendar" />);
    expect(screen.getByRole('group', { name: 'Google Calendar source' })).toBeInTheDocument();
  });

  it('draws no details gap when every child is off', () => {
    const { container } = render(
      <SettingsRow label="x">
        {false}
        {null}
      </SettingsRow>,
    );
    expect(container.querySelector('.mt-3')).toBeNull();
  });
});
