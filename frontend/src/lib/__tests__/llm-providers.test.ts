import { describe, it, expect } from 'vitest';
import { isCliProvider, isKeylessProvider, providerLabel } from '@/lib/llm-providers';

describe('llm-providers (specs/0085)', () => {
  it('knows which providers run through a subscription CLI', () => {
    expect(isCliProvider('claude-code')).toBe(true);
    expect(isCliProvider('codex-cli')).toBe(true);
    expect(isCliProvider('claude')).toBe(false);
    expect(isCliProvider('ollama')).toBe(false);
  });

  it('treats local and CLI providers as keyless, API providers as keyed', () => {
    for (const p of ['ollama', 'builtin-ai', 'claude-code', 'codex-cli']) {
      expect(isKeylessProvider(p)).toBe(true);
    }
    for (const p of ['claude', 'openai', 'groq', 'openrouter', 'custom-openai']) {
      expect(isKeylessProvider(p)).toBe(false);
    }
  });

  it('gives CLI providers a plain-language name', () => {
    expect(providerLabel('claude-code')).toBe('Claude Code');
    expect(providerLabel('codex-cli')).toBe('Codex');
    expect(providerLabel('claude')).toBe('Claude');
    expect(providerLabel('something-new')).toBe('something-new');
  });
});
