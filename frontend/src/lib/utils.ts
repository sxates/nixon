import { clsx, type ClassValue } from "clsx"
import { extendTailwindMerge } from "tailwind-merge"

/**
 * The named type scale and `shadow-card` from tailwind.config.js (docs/design-system.md).
 * tailwind-merge only knows Tailwind's built-in names, so without this it reads
 * `text-caption` as a text COLOUR and `cn('text-caption', 'text-muted-foreground')` drops
 * the size. Keep this list in step with `theme.extend.fontSize`.
 */
export const FONT_SIZE_TOKENS = [
  "micro",
  "label",
  "caption",
  "meta",
  "body",
  "lead",
  "heading",
  "title",
  "display",
] as const

const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [{ text: [...FONT_SIZE_TOKENS] }],
      shadow: [{ shadow: ["card"] }],
    },
  },
})

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/**
 * Detects if an error message indicates that Ollama is not installed or not running
 * @param errorMessage - The error message to check
 * @returns true if the error indicates Ollama is not installed/running
 */
/**
 * Configured/masked API-key status returned by the backend key getters
 * (spec 0030 WS2 / ADR-0009). Raw keys never cross IPC to the frontend.
 */
export interface ApiKeyStatus {
  configured: boolean;
  masked?: string | null;
}

/**
 * True when a field value is a masked display hint (e.g. "••••1234") rather
 * than a key the user typed. The backend ignores these on save, but the UI
 * also uses this to clear the field when the user chooses to replace a key.
 */
export function isMaskedApiKey(value: string | null | undefined): boolean {
  return !!value && value.startsWith('•');
}

/**
 * Local display hint for a key ("••••" + last 4 for long keys). Used so raw
 * keys the user just typed are not kept in long-lived state; already-masked
 * values pass through unchanged.
 */
export function maskApiKey(key: string): string {
  if (isMaskedApiKey(key)) return key;
  const mask = '•'.repeat(4);
  return key.length > 8 ? `${mask}${key.slice(-4)}` : mask;
}

/**
 * The field value to show for a stored key from an {@link ApiKeyStatus}:
 * the masked hint when configured, otherwise null.
 */
export function apiKeyHint(status: ApiKeyStatus | null | undefined): string | null {
  if (!status?.configured) return null;
  return status.masked ?? '•'.repeat(4);
}

export function isOllamaNotInstalledError(errorMessage: string): boolean {
  if (!errorMessage) return false;

  const lowerError = errorMessage.toLowerCase();

  // Check for common patterns that indicate Ollama is not installed or not running
  const patterns = [
    'cannot connect',
    'connection refused',
    'cli not found',
    'not in path',
    'ollama cli not found',
    'not found or not in path',
    'please check if the server is running',
    'please check if the ollama server is running',
    'econnrefused',
  ];

  return patterns.some(pattern => lowerError.includes(pattern));
}
