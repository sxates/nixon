'use client';

/**
 * EntryEditor (specs/0089): edit a timeline entry's points and decisions as lists of text
 * rows. Each row keeps its `t` (and a decision its `id`), so time links and attached
 * revisions survive an edit; a new decision goes up with an empty id and the backend
 * mints one. Without `entry` it writes a new manual entry.
 */

import { useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { X } from '@/components/ui/icons';
import { Input } from '@/components/ui/input';
import { addManualTopicEntry, editTopicEntry } from '@/lib/topics';
import type { EntryBody, TopicEntry } from '@/types';

interface Row {
  key: number;
  text: string;
  t: number | null;
  id: string;
}

let nextKey = 0;
const row = (text: string, t?: number | null, id = ''): Row => ({ key: nextKey++, text, t: t ?? null, id });
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface EntryEditorProps {
  topicId: string;
  /** The entry to edit; omit to add a manual entry. */
  entry?: TopicEntry;
  /** Called after a save or a cancel. */
  onDone: () => void;
}

export function EntryEditor({ topicId, entry, onDone }: EntryEditorProps) {
  const [bullets, setBullets] = useState<Row[]>(() =>
    entry ? entry.body.bullets.map((b) => row(b.text, b.t)) : [row('')],
  );
  const [decisions, setDecisions] = useState<Row[]>(() =>
    entry ? entry.body.decisions.map((d) => row(d.text, d.t, d.id)) : [],
  );
  const [saving, setSaving] = useState(false);

  const body: EntryBody = {
    bullets: bullets.filter((r) => r.text.trim()).map((r) => ({ text: r.text.trim(), t: r.t })),
    decisions: decisions
      .filter((r) => r.text.trim())
      .map((r) => ({ id: r.id, text: r.text.trim(), t: r.t })),
  };
  const empty = body.bullets.length === 0 && body.decisions.length === 0;

  const save = async () => {
    setSaving(true);
    try {
      if (entry) await editTopicEntry(entry.id, body);
      else await addManualTopicEntry(topicId, body);
      onDone();
    } catch (e) {
      console.error('Failed to save topic entry:', e);
      toast.error('Could not save the entry', { description: errorText(e) });
      setSaving(false);
    }
  };

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!empty && !saving) void save();
      }}
    >
      <RowList label="Points" noun="point" rows={bullets} onChange={setBullets} addLabel="Add point" />
      <RowList
        label="Decisions"
        noun="decision"
        rows={decisions}
        onChange={setDecisions}
        addLabel="Add decision"
      />
      <div className="flex items-center gap-2">
        <Button type="submit" size="sm" variant="brand" disabled={empty || saving}>
          Save
        </Button>
        <Button type="button" size="sm" variant="outline" disabled={saving} onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function RowList({
  label,
  noun,
  rows,
  onChange,
  addLabel,
}: {
  label: string;
  noun: string;
  rows: Row[];
  onChange: (rows: Row[]) => void;
  addLabel: string;
}) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="u-section-label pb-1">{label}</legend>
      {rows.map((r, i) => (
        <div key={r.key} className="flex items-center gap-1">
          <Input
            value={r.text}
            aria-label={`${noun[0].toUpperCase()}${noun.slice(1)} ${i + 1}`}
            onChange={(e) => onChange(rows.map((x) => (x.key === r.key ? { ...x, text: e.target.value } : x)))}
          />
          <IconButton
            type="button"
            size="sm"
            label={`Remove ${noun} ${i + 1}`}
            icon={<DeckIcon icon={X} size={14} />}
            onClick={() => onChange(rows.filter((x) => x.key !== r.key))}
          />
        </div>
      ))}
      <div>
        <Button type="button" size="xs" variant="dashed" onClick={() => onChange([...rows, row('')])}>
          {addLabel}
        </Button>
      </div>
    </fieldset>
  );
}
