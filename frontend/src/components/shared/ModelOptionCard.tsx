'use client';

/**
 * One transcription model in a picker (Whisper, Parakeet): an `OptionCard` for the choice,
 * with the card's own actions (download, retry, delete) beside it rather than nested in
 * it, and the download progress under it. Used inside an `OptionCardGroup` whose value is
 * the selected model. A model that isn't on disk yet can't be chosen, so its card is
 * disabled until it is.
 *
 * Replaces the two hand-built `border-2` cards in WhisperModelManager and
 * ParakeetModelManager (design-system wave C).
 */

import * as React from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { Trash2 } from '@/components/ui/icons';
import { OptionCard } from '@/components/ui/option-card';
import { Progress } from '@/components/ui/progress';
import { StatusDot } from '@/components/ui/status-dot';

/**
 * Both model libraries' `ModelStatus`: `'Available'`, `'Missing'`, or a one-key object
 * (`{ Downloading: pct }`, `{ Error: … }`, `{ Corrupted: … }`).
 */
export type ModelCardStatus = unknown;

export interface ModelOptionCardProps {
  value: string;
  title: React.ReactNode;
  description?: React.ReactNode;
  /** Tier / quantization badges on the title line. */
  badges?: React.ReactNode;
  isRecommended?: boolean;
  status: ModelCardStatus;
  sizeMb?: number;
  formatSize: (mb: number) => string;
  onDownload: () => void;
  onCancel: () => void;
  onDelete: () => void;
}

function isObj(s: unknown, key: string): s is Record<string, unknown> {
  return typeof s === 'object' && s !== null && key in s;
}

export function ModelOptionCard({
  value,
  title,
  description,
  badges,
  isRecommended = false,
  status,
  sizeMb,
  formatSize,
  onDownload,
  onCancel,
  onDelete,
}: ModelOptionCardProps) {
  const isAvailable = status === 'Available';
  const isMissing = status === 'Missing';
  const isError = isObj(status, 'Error');
  const isCorrupted = isObj(status, 'Corrupted');
  const downloadProgress = isObj(status, 'Downloading') ? (status.Downloading as number) : null;

  return (
    <div className="group space-y-2">
      <div className="flex items-center gap-2">
        <OptionCard
          value={value}
          disabled={!isAvailable}
          className="flex-1"
          title={title}
          description={description}
          trailing={
            <>
              {badges}
              {isRecommended ? <Badge tone="brand">Recommended</Badge> : null}
              {isAvailable ? (
                <span className="inline-flex items-center gap-1.5 text-xs font-medium text-success">
                  <StatusDot tone="success" size={6} />
                  Ready
                </span>
              ) : null}
            </>
          }
        />

        {isAvailable ? (
          <IconButton
            variant="ghost"
            label="Delete model to free up space"
            icon={<DeckIcon icon={Trash2} />}
            onClick={onDelete}
            className="opacity-0 hover:text-destructive focus-visible:opacity-100 group-hover:opacity-100"
          />
        ) : null}

        {isMissing ? (
          <Button variant="outline" size="sm" onClick={onDownload}>
            Download
          </Button>
        ) : null}

        {downloadProgress === null && isError ? (
          <Button variant="outline" size="sm" onClick={onDownload}>
            Retry
          </Button>
        ) : null}

        {isCorrupted ? (
          <>
            <Button variant="outline" size="sm" className="text-destructive hover:text-destructive" onClick={onDelete}>
              Delete
            </Button>
            <Button variant="outline" size="sm" onClick={onDownload}>
              Re-download
            </Button>
          </>
        ) : null}
      </div>

      {downloadProgress !== null ? (
        <div className="px-1">
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-xs font-medium text-brand">
              Downloading... <span className="font-semibold tabular-nums">{Math.round(downloadProgress)}%</span>
            </span>
            <Button variant="ghost" size="xs" onClick={onCancel} title="Cancel download">
              Cancel
            </Button>
          </div>
          <Progress value={downloadProgress} aria-label={`Downloading ${typeof title === 'string' ? title : value}`} />
          <p className="mt-1 text-xs text-muted-foreground">
            {sizeMb ? `${formatSize((sizeMb * downloadProgress) / 100)} / ${formatSize(sizeMb)}` : 'Downloading...'}
          </p>
        </div>
      ) : null}
    </div>
  );
}
