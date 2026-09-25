import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import {
  VirtualizedTranscriptView,
  type InlineSpeakerAssignment,
} from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { TranscriptSegmentData } from '@/types';

// Owner feedback: with several lines selected, the reassign control sat at the very bottom
// of the transcript — off-screen on a long meeting, because the page column (not the
// transcript) owns the scroll. It is now a sticky bar pinned to the bottom of the visible
// viewport, the transcript gets bottom clearance while it is up, and Escape clears.

const SEGMENTS: TranscriptSegmentData[] = Array.from({ length: 6 }, (_, i) => ({
  id: `seg-${i}`,
  timestamp: i * 3,
  text: `line ${i}`,
  speaker: 'spk_0',
  speakerName: 'Speaker 1',
}));

function assignment(): InlineSpeakerAssignment {
  return {
    attendees: [],
    people: [],
    onAssignAttendee: vi.fn(),
    onAssignPerson: vi.fn(),
    speakers: [
      { speakerKey: 'spk_0', displayName: 'Speaker 1' },
      { speakerKey: 'spk_1', displayName: 'Speaker 2' },
    ],
    onReassignSegment: vi.fn(),
    onReassignSegments: vi.fn().mockResolvedValue(true),
    onCreateSpeaker: vi.fn(),
  };
}

function renderView() {
  return render(
    <TooltipProvider>
      <VirtualizedTranscriptView segments={SEGMENTS} disableAutoScroll assignment={assignment()} />
    </TooltipProvider>,
  );
}

function selectTwo() {
  const boxes = screen.getAllByRole('checkbox');
  fireEvent.click(boxes[1]);
  fireEvent.click(boxes[3]);
}

const bar = () => screen.queryByRole('toolbar', { name: /selected transcript lines/i, hidden: true });
const content = () =>
  screen.getByRole('region', { name: /meeting transcript/i }).firstElementChild as HTMLElement;

describe('transcript selection action bar', () => {
  it('is absent until a line is selected', () => {
    renderView();
    expect(bar()).toBeNull();
    expect(content().className).not.toMatch(/pb-16/);
  });

  it('floats in a sticky bottom anchor with the count, reassign and clear', () => {
    renderView();
    selectTwo();
    const toolbar = bar();
    expect(toolbar).not.toBeNull();
    expect(toolbar!.textContent).toContain('2 lines selected');
    expect(screen.getByRole('button', { name: /reassign to/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /clear selection/i })).toBeInTheDocument();

    // Sticky to the bottom of whatever scrolls (the page column), and OUTSIDE the transcript's
    // own scroll box — inside it the bar would stick to a box that never scrolls.
    const anchor = toolbar!.closest('[data-selection-bar-anchor]') as HTMLElement;
    expect(anchor.className).toMatch(/\bsticky\b/);
    expect(anchor.className).toMatch(/\bbottom-0\b/);
    expect(screen.getByRole('region', { name: /meeting transcript/i }).contains(anchor)).toBe(false);

    // The foot of the transcript gets clearance so the bar covers no line at the end.
    expect(content().className).toMatch(/pb-16/);
  });

  it('Escape clears the selection and removes the bar', () => {
    renderView();
    selectTwo();
    expect(bar()).not.toBeNull();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(bar()).toBeNull();
    expect(screen.getAllByRole('checkbox').every((b) => b.getAttribute('aria-checked') === 'false')).toBe(true);
    expect(content().className).not.toMatch(/pb-16/);
  });

  it('Escape from focus inside the transcript clears', () => {
    renderView();
    selectTwo();
    const box = screen.getAllByRole('checkbox')[1];
    box.focus();
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(bar()).toBeNull();
  });

  it('Escape pressed in the sidebar or the notes editor leaves the selection alone', () => {
    renderView();
    selectTwo();
    const sidebarLink = document.createElement('button');
    sidebarLink.textContent = 'Today';
    const notes = document.createElement('div');
    notes.setAttribute('contenteditable', 'true');
    document.body.append(sidebarLink, notes);
    fireEvent.keyDown(sidebarLink, { key: 'Escape' });
    expect(bar()).not.toBeNull();
    fireEvent.keyDown(notes, { key: 'Escape' });
    expect(bar()).not.toBeNull();
    sidebarLink.remove();
    notes.remove();
  });

  it('Escape on the body is ignored while the transcript tab is hidden', () => {
    const { container } = renderView();
    selectTwo();
    (container.firstElementChild as HTMLElement).style.display = 'none';
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(bar()).not.toBeNull();
  });

  it('Escape inside a listbox or combobox is left to it, even within the transcript', () => {
    renderView();
    selectTwo();
    const region = screen.getByRole('region', { name: /meeting transcript/i });
    for (const role of ['listbox', 'combobox']) {
      const el = document.createElement('div');
      el.setAttribute('role', role);
      el.tabIndex = 0;
      region.appendChild(el);
      fireEvent.keyDown(el, { key: 'Escape' });
      expect(bar()).not.toBeNull();
      el.remove();
    }
  });

  it('Escape inside a text field is left to the field', () => {
    renderView();
    selectTwo();
    const input = document.createElement('input');
    document.body.appendChild(input);
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(bar()).not.toBeNull();
    input.remove();
  });

  it('lifts toasts above the bar only while it is shown', () => {
    renderView();
    selectTwo();
    expect(document.documentElement.style.getPropertyValue('--toast-lift')).not.toBe('');
    fireEvent.click(screen.getByRole('button', { name: /clear selection/i }));
    expect(document.documentElement.style.getPropertyValue('--toast-lift')).toBe('');
  });
});
