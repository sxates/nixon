/**
 * The icon allowlist (docs/design-system.md §Icons). Screens import glyphs from here, never
 * straight from `lucide-react`, and draw them through `DeckIcon`. Adding a glyph to this
 * file is a reviewed design decision: icons are functional, not decorative.
 */

// Functional glyphs: back, chevrons, close, check, overflow, play/pause, spinner, star
// toggle, refresh, icon-only trash, add, edit — and Sparkles, which marks an AI action.
export {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Loader2,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Sparkles,
  Star,
  Trash2,
  X,
} from 'lucide-react';

// Dropdown-menu leading icons (`DropdownMenuItem icon={…}`) in use today.
export { Circle, EyeOff, MicOff, Radio, RotateCcw, UserCheck, UserPlus, Waves } from 'lucide-react';

export type { LucideIcon } from 'lucide-react';
