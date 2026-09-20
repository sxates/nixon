'use client';

/**
 * The Summary tab's resolved model row (specs/0067 W1).
 *
 * What it replaces: a provider menu plus four built-in models whose names — "Qwen 3.5 2B",
 * "gemma3:4b" — are not a choice a normal user can make. Nixon already made it. On first
 * run `database/commands.rs` writes `recommend_summary_model(ram)` as the default config,
 * so the menu was asking a question the app had already answered, in vocabulary only an AI
 * hobbyist reads.
 *
 * This row says what is in use and why, and nothing else. The full controls come back via
 * Settings → General → "Show advanced options", or on their own if the configured model is
 * not the recommended one (`isAdvancedRowVisible`) — a choice someone made deliberately
 * must not vanish behind a switch they have never touched.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SettingsGroup, SettingsRow } from '@/components/ui/settings';

interface BuiltInModel {
  name: string;
  display_name: string;
}

/** Strips the marketing parenthetical: "Qwen 3.5 4B (High Quality)" → "Qwen 3.5 4B". */
function plainName(displayName: string): string {
  return displayName.replace(/\s*\([^)]*\)\s*$/, '').trim();
}

export function SummaryModelRow({ model }: { model: string }) {
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

  return (
    <SettingsGroup>
      <SettingsRow
        label="Summary model"
        description="Chosen to suit this Mac's memory. Summaries are written here, on your machine — nothing is sent anywhere."
        control={
          <span className="u-section-label text-[11px] text-engrave" data-testid="resolved-summary-model">
            {label ?? '…'}
          </span>
        }
      />
    </SettingsGroup>
  );
}
