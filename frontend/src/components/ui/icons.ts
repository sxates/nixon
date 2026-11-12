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

// Functional: the magnifier inside a search field (People directory).
export { Search } from 'lucide-react';

// Functional, Settings (design-system wave C): the API-key field's icon-only lock and
// show/hide toggles, and the model combobox trigger's up/down chevron.
export { ChevronsUpDown, Eye, Lock, Unlock } from 'lucide-react';

// Dropdown-menu leading icons (`DropdownMenuItem icon={…}`) in use today.
export { Circle, EyeOff, MicOff, Radio, RotateCcw, UserCheck, UserPlus, Waves } from 'lucide-react';

// Icon-only controls whose glyph IS the label (the tooltip carries the words): search,
// import audio, the agenda's jump-to-date picker, the manual-sort drag handle, and the
// sidebar's update download / restart glyph.
export { CalendarDays, Download, FileAudio, GripVertical, RotateCw } from 'lucide-react';

// Sidebar navigation. The collapsed rail is icon-only, so each destination's glyph is its
// only visible wayfinding; the hamburger toggles the rail.
export { CheckSquare, Menu, MessageSquare, Settings, Users } from 'lucide-react';

// Command palette (⌘K) item leading icons — the palette's rows are menu items.
export {
  AudioLines,
  Calendar,
  FileText,
  LayoutList,
  ListPlus,
  Mic,
  NotebookPen,
} from 'lucide-react';

export type { LucideIcon } from 'lucide-react';
