/**
 * People IPC wrappers (specs/0038 dogfood feedback #3).
 *
 * Thin `invoke` helpers for the People directory + the dedicated person-detail page.
 * The list/create/update/star commands are called inline where they're used; this
 * module holds the single-person read the detail page needs.
 */

import { invoke } from '@tauri-apps/api/core';
import type { Person } from '@/types';

/** Load one person by id, or `null` if they've been forgotten. */
export function getPerson(id: string): Promise<Person | null> {
  return invoke<Person | null>('api_get_person', { id });
}
