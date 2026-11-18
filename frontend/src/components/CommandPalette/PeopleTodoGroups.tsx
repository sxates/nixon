'use client';

/**
 * The ⌘K palette's People and To-Dos groups (v0.18 feedback). Both are fetched on every
 * palette open, like Topics, and show only for a typed query: an empty palette would
 * otherwise list the whole directory and the whole To-Do list. Matching runs cmdk's own
 * `defaultFilter` over the same value + keywords the rows register, so cmdk's filter (a
 * 1-character query) and ours (the backend-search tier) agree, and each group keeps its
 * best few matches.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { defaultFilter } from 'cmdk';
import { CommandGroup, CommandItem } from '@/components/ui/command';
import { CheckSquare, Users } from '@/components/ui/icons';
import { listPeople } from '@/lib/people';
import { deserializeMentions } from '@/lib/todo-parse';
import type { Person, TodoRow } from '@/types';

/** How many matches each group shows. */
export const PALETTE_GROUP_CAP = 5;

interface Surface {
  value: string;
  keywords: string[];
}

/** The best `cap` items for `query`, best first (ties keep their incoming order). */
export function topMatches<T>(
  items: T[],
  query: string,
  surface: (item: T) => Surface,
  cap = PALETTE_GROUP_CAP,
): T[] {
  if (!query) return [];
  return items
    .map((item) => {
      const s = surface(item);
      return { item, score: defaultFilter(s.value, query, s.keywords) };
    })
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, cap)
    .map((x) => x.item);
}

export const personSurface = (p: Person): Surface => ({
  value: `${p.displayName} ${p.id}`,
  keywords: [p.displayName, p.email ?? '', p.role ?? '', 'person', 'people'].filter(Boolean),
});

/** A To-Do's description with `@[Name](person:id)` tokens shown as `@Name`. */
export const todoText = (t: TodoRow) => deserializeMentions(t.description).text;

export const todoSurface = (t: TodoRow): Surface => {
  const text = todoText(t);
  return { value: `${text} ${t.id}`, keywords: [text, 'to-do', 'todo'] };
};

/** People and the To-Do list, refetched each time the palette opens. A failed fetch keeps
 *  the last list and retries on the next open. */
export function usePaletteEntities(open: boolean): { people: Person[]; todos: TodoRow[] } {
  const [people, setPeople] = useState<Person[]>([]);
  const [todos, setTodos] = useState<TodoRow[]>([]);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    listPeople()
      .then((r) => !cancelled && setPeople(Array.isArray(r) ? r : []))
      .catch((err) => console.error('[CommandPalette] Failed to load people:', err));
    invoke<TodoRow[]>('api_list_todo_view', { view: 'list' })
      .then((r) => !cancelled && setTodos(Array.isArray(r) ? r : []))
      .catch((err) => console.error('[CommandPalette] Failed to load to-dos:', err));
    return () => {
      cancelled = true;
    };
  }, [open]);
  return { people, todos };
}

export function PeopleGroup({ people, onOpen }: { people: Person[]; onOpen: (id: string) => void }) {
  if (people.length === 0) return null;
  return (
    <CommandGroup heading="People">
      {people.map((p) => {
        const s = personSurface(p);
        return (
          <CommandItem key={p.id} value={s.value} keywords={s.keywords} onSelect={() => onOpen(p.id)}>
            <Users />
            <span className="min-w-0 flex-1 truncate">{p.displayName}</span>
            {p.role && (
              <span className="ml-auto max-w-[40%] flex-shrink-0 truncate text-xs text-muted-foreground">
                {p.role}
              </span>
            )}
          </CommandItem>
        );
      })}
    </CommandGroup>
  );
}

export function TodosGroup({ todos, onOpen }: { todos: TodoRow[]; onOpen: (id: string) => void }) {
  if (todos.length === 0) return null;
  return (
    <CommandGroup heading="To-Dos">
      {todos.map((t) => {
        const s = todoSurface(t);
        return (
          <CommandItem key={t.id} value={s.value} keywords={s.keywords} onSelect={() => onOpen(t.id)}>
            <CheckSquare />
            <span className="min-w-0 flex-1 truncate">{todoText(t)}</span>
          </CommandItem>
        );
      })}
    </CommandGroup>
  );
}
