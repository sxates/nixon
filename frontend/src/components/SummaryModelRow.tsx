'use client';

/**
 * The human name of the summary model in use (specs/0067).
 *
 * Nixon picks the model from the machine's RAM on first run, so the Summary tab states it
 * rather than asking. The ids it is picked by — `qwen3.5:4b`, `gemma3:1b` — are not names
 * anyone outside the hobby reads, so this resolves them through the catalogue the backend
 * already exposes and strips the marketing parenthetical.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface BuiltInModel {
  name: string;
  display_name: string;
}

/** "Qwen 3.5 4B (High Quality)" → "Qwen 3.5 4B". */
function plainName(displayName: string): string {
  return displayName.replace(/\s*\([^)]*\)\s*$/, '').trim();
}

export function SummaryModelName({ model }: { model: string }) {
  const [label, setLabel] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<BuiltInModel[]>('builtin_ai_list_models')
      .then((models) => {
        if (cancelled) return;
        const match = models.find((m) => m.name === model);
        setLabel(match ? plainName(match.display_name) : model);
      })
      // The id is a poor label but an honest one; better than an empty row.
      .catch(() => !cancelled && setLabel(model));
    return () => {
      cancelled = true;
    };
  }, [model]);

  return <>{label ?? '…'}</>;
}
