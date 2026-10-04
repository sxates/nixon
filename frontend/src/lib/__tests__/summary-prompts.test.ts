import { describe, it, expect } from 'vitest';
import {
  isPersistableMeetingId,
  promptSourceLabel,
  shouldConfirmPromptChange,
  type MeetingPromptState,
} from '@/lib/summary-prompts';

const state = (over: Partial<MeetingPromptState>): MeetingPromptState => ({
  source: 'default',
  prompt_id: 'p1',
  prompt_name: 'Weekly',
  extract_action_items: true,
  custom_body: null,
  custom_extract_action_items: null,
  has_series: false,
  ...over,
});

describe('isPersistableMeetingId', () => {
  it('accepts real ids and rejects fabricated ones', () => {
    expect(isPersistableMeetingId('meeting-3f2a9c1e-89ab-4cde-8123-456789abcdef')).toBe(true);
    expect(isPersistableMeetingId('meeting-123456')).toBe(false);
    expect(isPersistableMeetingId('intro-call')).toBe(false);
    expect(isPersistableMeetingId(null)).toBe(false);
    expect(isPersistableMeetingId(undefined)).toBe(false);
    expect(isPersistableMeetingId('  ')).toBe(false);
  });
});

describe('promptSourceLabel', () => {
  it.each([
    [state({ source: 'custom' }), 'Custom (this meeting)'],
    [state({ source: 'meeting' }), 'Weekly'],
    [state({ source: 'meeting', prompt_name: null }), 'Prompt'],
    [state({ source: 'series' }), 'Weekly (series)'],
    [state({ source: 'default' }), 'Weekly'],
    [state({ source: 'default', prompt_name: null }), 'Default'],
    [state({ source: 'fallback', prompt_name: null }), 'Default'],
    [null, 'Default'],
  ])('labels %j as %s', (s, expected) => {
    expect(promptSourceLabel(s)).toBe(expected);
  });
});

describe('shouldConfirmPromptChange', () => {
  it('confirms only with an existing summary and a different prompt', () => {
    expect(shouldConfirmPromptChange('p2', state({}), true)).toBe(true);
    expect(shouldConfirmPromptChange('p1', state({}), true)).toBe(false);
    expect(shouldConfirmPromptChange('p2', state({}), false)).toBe(false);
    expect(shouldConfirmPromptChange('p2', null, true)).toBe(true);
  });
  it('confirms when leaving a custom one-off even for the same id', () => {
    expect(shouldConfirmPromptChange('p1', state({ source: 'custom' }), true)).toBe(true);
    expect(shouldConfirmPromptChange('p1', state({ source: 'custom' }), false)).toBe(false);
  });
});
