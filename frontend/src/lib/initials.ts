/**
 * The one initials helper (docs/design-system.md §2: `Avatar` owns "the only `initials()`").
 * It lives in lib/ so lib code (`day-agenda`) can re-export it without importing a
 * component; screens import it from `@/components/ui/avatar`.
 *
 * - "Priya Sharma" → "PS", "Ana Maria de Souza" → "AS" (first + last word)
 * - "priya" → "P" (a single word is one letter, the way directory avatars read)
 * - "jordan.lee@example.com" → "JL" (an email uses its local part, split on . _ -)
 * - "" → "?"
 *
 * Before wave 0b there were two: PersonAvatar's ("Madonna" → "M") and day-agenda's
 * ("Madonna" → "MA", email-aware). This keeps the email handling and the one-letter rule.
 */
export function initials(nameOrEmail: string): string {
  const trimmed = nameOrEmail.trim();
  if (!trimmed) return '?';
  const base = trimmed.includes('@') ? trimmed.split('@')[0]! : trimmed;
  const parts = base.split(/[\s._-]+/).filter(Boolean);
  if (parts.length === 0) return base.charAt(0).toUpperCase() || '?';
  if (parts.length === 1) return parts[0]!.charAt(0).toUpperCase();
  return (parts[0]!.charAt(0) + parts[parts.length - 1]!.charAt(0)).toUpperCase();
}
