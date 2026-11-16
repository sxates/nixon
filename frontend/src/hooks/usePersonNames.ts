'use client';

import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { Person } from '@/types';

/**
 * person id → display name, for rendering `@[Name](person:<id>)` tokens and "adopted from"
 * labels in a to-do list. Empty until loaded (and on failure), so a list renders either way.
 */
export function usePersonNames(): Map<string, string> {
  const [names, setNames] = useState<Map<string, string>>(() => new Map());
  useEffect(() => {
    let live = true;
    invoke<Person[]>('api_list_people')
      .then((people) => {
        if (live && Array.isArray(people)) setNames(new Map(people.map((p) => [p.id, p.displayName])));
      })
      .catch((err) => console.error('Failed to load people:', err));
    return () => {
      live = false;
    };
  }, []);
  return names;
}
