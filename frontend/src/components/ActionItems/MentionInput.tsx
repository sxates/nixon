'use client';

/**
 * MentionInput (specs/0088 §Quick add "Mentions") — a single-line text field where typing
 * `@` + letters opens a People list; Enter / click inserts `@Name ` and remembers the pick.
 * The value in and out is the SERIALIZED description (`@[Name](person:<id>)` tokens): the
 * field shows plain `@Name`, and `serializeMentions` turns picked names back into tokens on
 * submit. An `@word` that was never picked stays plain text, and a person is never created
 * from here. The backend syncs the people link from the tokens, so there is no extra IPC.
 *
 * Keys: ↑/↓ move, Enter picks, Esc closes the list only (a second Esc is the caller's
 * cancel), a space with no remaining match closes it. The list is built from `groups`
 * (`{ heading, items }[]`) so a Topics group can sit above People without a rewrite.
 */

import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Command, CommandGroup, CommandItem, CommandList } from '@/components/ui/command';
import { Input } from '@/components/ui/input';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { deserializeMentions, serializeMentions, type PickedMention } from '@/lib/todo-parse';
import type { Person } from '@/types';

export interface MentionGroupItem {
  key: string;
  label: string;
  /** Called when the row is picked; returns the mention to remember. */
  mention: PickedMention;
}
export interface MentionGroup {
  heading: string;
  items: MentionGroupItem[];
}

const MAX_RESULTS = 8;
// `@` at the start or after whitespace, then the query up to the caret. Spaces are allowed
// so "@Sam R" keeps matching "Sam Rivera"; the list closes when nothing matches.
const ACTIVE_MENTION = /(^|\s)@([^@]*)$/;

interface MentionInputProps {
  /** Serialized description. */
  value: string;
  /** Enter — receives the serialized text (tokens in place). */
  onSubmit: (serialized: string) => void;
  /** Esc with the list closed. */
  onCancel?: () => void;
  /** Every edit, serialized — for callers that track a draft (button state). */
  onChange?: (serialized: string) => void;
  /** Blur submits too (inline edit surfaces). */
  submitOnBlur?: boolean;
  placeholder?: string;
  ariaLabel?: string;
  className?: string;
  autoFocus?: boolean;
}

export function MentionInput({
  value,
  onSubmit,
  onCancel,
  onChange,
  submitOnBlur = false,
  placeholder,
  ariaLabel,
  className,
  autoFocus,
}: MentionInputProps) {
  const initial = useMemo(() => deserializeMentions(value), []); // eslint-disable-line react-hooks/exhaustive-deps
  const [text, setText] = useState(initial.text);
  const [picked, setPicked] = useState<PickedMention[]>(initial.picked);
  const [caret, setCaret] = useState(initial.text.length);
  const [people, setPeople] = useState<Person[]>([]);
  const [active, setActive] = useState(0);
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const pendingCaret = useRef<number | null>(null);

  // A different `value` from outside (cleared after an add, saved elsewhere) replaces the field.
  useEffect(() => {
    if (value === serializeMentions(text, picked)) return;
    const d = deserializeMentions(value);
    setText(d.text);
    setPicked(d.picked);
    setCaret(d.text.length);
  }, [value]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const result = await invoke<Person[]>('api_list_people_ranked');
        if (!cancelled) setPeople(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('Failed to load people for mentions:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (pendingCaret.current != null && inputRef.current) {
      inputRef.current.setSelectionRange(pendingCaret.current, pendingCaret.current);
      pendingCaret.current = null;
    }
  });

  const match = ACTIVE_MENTION.exec(text.slice(0, caret));
  const atIndex = match ? match.index + match[1].length : -1;
  const query = match ? match[2].toLowerCase() : '';

  const groups: MentionGroup[] = useMemo(() => {
    if (!match) return [];
    const items = people
      .filter((p) => p.displayName.toLowerCase().includes(query))
      .slice(0, MAX_RESULTS)
      .map((p) => ({
        key: p.id,
        label: p.displayName,
        mention: { name: p.displayName, personId: p.id },
      }));
    return items.length ? [{ heading: 'People', items }] : [];
  }, [people, query, match?.[0]]); // eslint-disable-line react-hooks/exhaustive-deps

  const flat = groups.flatMap((g) => g.items);
  const open = flat.length > 0 && dismissedAt !== atIndex;
  const activeIndex = Math.min(active, Math.max(flat.length - 1, 0));

  const emit = (nextText: string, nextPicked: PickedMention[]) => {
    onChange?.(serializeMentions(nextText, nextPicked));
  };

  const pick = (item: MentionGroupItem) => {
    const before = text.slice(0, atIndex);
    const insert = `@${item.mention.name} `;
    const nextText = before + insert + text.slice(caret);
    const nextPicked = picked.some((p) => p.personId === item.mention.personId)
      ? picked
      : [...picked, item.mention];
    setText(nextText);
    setPicked(nextPicked);
    pendingCaret.current = before.length + insert.length;
    setCaret(before.length + insert.length);
    setActive(0);
    emit(nextText, nextPicked);
  };

  const submit = () => onSubmit(serializeMentions(text, picked));

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (open) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const n = flat.length;
        setActive((activeIndex + (e.key === 'ArrowDown' ? 1 : n - 1)) % n);
        return;
      }
      if (e.key === 'Enter') {
        e.preventDefault();
        pick(flat[activeIndex]);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        setDismissedAt(atIndex);
        return;
      }
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape' && onCancel) {
      e.preventDefault();
      onCancel();
    }
  };

  let flatIndex = -1;
  return (
    <Popover open={open}>
      <PopoverAnchor asChild>
        <Input
          ref={inputRef}
          value={text}
          autoFocus={autoFocus}
          placeholder={placeholder}
          aria-label={ariaLabel ?? placeholder}
          aria-autocomplete="list"
          aria-expanded={open}
          className={className}
          onChange={(e) => {
            setText(e.target.value);
            setCaret(e.target.selectionStart ?? e.target.value.length);
            setActive(0);
            setDismissedAt(null);
            emit(e.target.value, picked);
          }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? e.currentTarget.value.length)}
          onKeyDown={onKeyDown}
          onBlur={() => {
            if (submitOnBlur) submit();
          }}
        />
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-64 p-0"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        // Keep focus in the field so picking with the mouse doesn't blur (and submit) it.
        onMouseDown={(e) => e.preventDefault()}
      >
        <Command shouldFilter={false} value={`mention-${activeIndex}`}>
          <CommandList>
            {groups.map((group) => (
              <CommandGroup key={group.heading} heading={group.heading}>
                {group.items.map((item) => {
                  const idx = ++flatIndex;
                  return (
                    <CommandItem key={item.key} value={`mention-${idx}`} onSelect={() => pick(item)}>
                      {item.label}
                    </CommandItem>
                  );
                })}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
