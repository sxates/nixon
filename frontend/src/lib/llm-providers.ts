/**
 * Summary-provider helpers (specs/0085).
 *
 * "Subscription" providers run the user's own installed CLI (Claude Code today, Codex
 * later) under their own login: no API key, nothing to download. One place decides what
 * that means so Settings, the summary row and the key handling cannot drift apart.
 */

export type CliProviderId = 'claude-code' | 'codex-cli';

const CLI_PROVIDERS: ReadonlySet<string> = new Set<CliProviderId>(['claude-code', 'codex-cli']);

/** Providers that run through the user's own subscription CLI. */
export function isCliProvider(provider: string): provider is CliProviderId {
  return CLI_PROVIDERS.has(provider);
}

/** Providers that never take an API key: local ones, and the subscription CLIs. */
export function isKeylessProvider(provider: string): boolean {
  return provider === 'ollama' || provider === 'builtin-ai' || isCliProvider(provider);
}

const LABELS: Record<string, string> = {
  'claude-code': 'Claude Code',
  'codex-cli': 'Codex',
  claude: 'Claude',
  openai: 'OpenAI',
  groq: 'Groq',
  openrouter: 'OpenRouter',
  ollama: 'Ollama',
  'builtin-ai': 'Built-in AI',
  'custom-openai': 'Custom Server',
};

export function providerLabel(provider: string): string {
  return LABELS[provider] ?? provider;
}

/** "sonnet" -> "Sonnet". The CLI model ids are single lowercase words. */
export function cliModelLabel(model: string): string {
  return model ? model.charAt(0).toUpperCase() + model.slice(1) : model;
}
