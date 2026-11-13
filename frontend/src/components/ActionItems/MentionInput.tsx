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
 * (`{ heading, items }[]`): Topics (specs/0089, open topics passed in by the caller) sit
 * above People. Picking a topic is not a mention: it removes the `@query` text and calls
 * `onTopicChange`, because a to-do holds at most one topic and the caller owns the tag.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ChangeEvent, type ComponentProps, forwardRef, type KeyboardEvent, type SyntheticEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Command, CommandGroup, CommandItem, CommandList } from '@/components/ui/command';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { deserializeMentions, serializeMentions, type PickedMention } from '@/lib/todo-parse';
import type { Person } from '@/types';

export interface MentionGroupItem {
  key: string;
  label: string;
  /** A person row: the mention to remember when picked. */
  mention?: PickedMention;
  /** A topic row (specs/0089): picking it tags the to-do instead of inserting text. */
  topic?: MentionTopic;
  /** Muted trailing hint (e.g. "Replaces Atlas CRM rebuild"). */
  hint?: string;
}
export interface MentionTopic {
  id: string;
  name: string;
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
  /** Primitive variant/size passed through: `bare` is the inline-edit surface. */
  variant?: 'default' | 'bare';
  size?: 'title';
  /** Auto-growing textarea (the detail-page title). Enter still submits; Shift+Enter is
   *  ignored — titles never hold newlines. */
  multiline?: boolean;
  /** Bump to force the field back to `value` even when `value` itself didn't change
   *  (after a save that normalised the text, or a cancel). */
  resetKey?: number;
  /** Open topics offered under `@` (above People). Omit for a person-only picker. */
  topics?: MentionTopic[];
  /** The topic the to-do already has: other topics then show "Replaces {name}". */
  topicId?: string | null;
  onTopicChange?: (topic: MentionTopic | null) => void;
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
  multiline = false,
  resetKey = 0,
  variant = 'default',
  size,
  topics,
  topicId = null,
  onTopicChange,
}: MentionInputProps) {
  // cmdk owns the listbox's id, so read it back from the DOM once the list mounts.
  const [listId, setListId] = useState<string | undefined>(undefined);
  const listRef = useRef<HTMLDivElement | null>(null);
  const [activeOptionId, setActiveOptionId] = useState<string | undefined>(undefined);
  const initial = useMemo(() => deserializeMentions(value), []); // eslint-disable-line react-hooks/exhaustive-deps
  const [text, setText] = useState(initial.text);
  const [picked, setPicked] = useState<PickedMention[]>(initial.picked);
  const [caret, setCaret] = useState(initial.text.length);
  const [people, setPeople] = useState<Person[]>([]);
  const [active, setActive] = useState(0);
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  const inputRef = useRef<HTMLInputElement & HTMLTextAreaElement>(null);
  // Set by Enter/Esc so the blur that follows (unmount, focus move) can't submit again.
  const settled = useRef(false);
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

  const firstReset = useRef(true);
  useEffect(() => {
    if (firstReset.current) {
      firstReset.current = false;
      return;
    }
    const d = deserializeMentions(value);
    setText(d.text);
    setPicked(d.picked);
    setCaret(d.text.length);
    setDismissedAt(null);
  }, [resetKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Show each picked person's CURRENT name (a rename since the token was written).
  useEffect(() => {
    if (!people.length) return;
    const names = new Map(people.map((p) => [p.id, p.displayName]));
    const stale = picked.filter((p) => names.has(p.personId) && names.get(p.personId) !== p.name);
    if (!stale.length) return;
    let nextText = text;
    const nextPicked = picked.map((p) => {
      const cur = names.get(p.personId);
      if (!cur || cur === p.name) return p;
      nextText = nextText.split(`@${p.name}`).join(`@${cur}`);
      return { ...p, name: cur };
    });
    setText(nextText);
    setPicked(nextPicked);
  }, [people]); // eslint-disable-line react-hooks/exhaustive-deps

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
    const current = topicId ? topics?.find((t) => t.id === topicId) : undefined;
    const topicItems = (topics ?? [])
      .filter((t) => t.name.toLowerCase().includes(query))
      .slice(0, MAX_RESULTS)
      .map((t) => ({
        key: `topic-${t.id}`,
        label: t.name,
        topic: t,
        hint: current && current.id !== t.id ? `Replaces ${current.name}` : undefined,
      }));
    const items = people
      .filter((p) => p.displayName.toLowerCase().includes(query))
      .slice(0, MAX_RESULTS)
      .map((p) => ({
        key: p.id,
        label: p.displayName,
        mention: { name: p.displayName, personId: p.id },
      }));
    const out: MentionGroup[] = [];
    if (topicItems.length) out.push({ heading: 'Topics', items: topicItems });
    if (items.length) out.push({ heading: 'People', items });
    return out;
  }, [people, topics, topicId, query, match?.[0]]); // eslint-disable-line react-hooks/exhaustive-deps

  const flat = groups.flatMap((g) => g.items);
  const open = flat.length > 0 && dismissedAt !== atIndex;
  const activeIndex = Math.min(active, Math.max(flat.length - 1, 0));

  useEffect(() => {
    const opts = listRef.current?.querySelectorAll('[role="option"]');
    setActiveOptionId(open ? (opts?.[activeIndex] as HTMLElement | undefined)?.id : undefined);
  }, [open, activeIndex, flat.length, listId]);

  const emit = (nextText: string, nextPicked: PickedMention[]) => {
    onChange?.(serializeMentions(nextText, nextPicked));
  };

  const pick = (item: MentionGroupItem) => {
    if (item.topic) {
      // A topic pick leaves no text behind: drop `@query`, keep the rest.
      const nextText = text.slice(0, atIndex) + text.slice(caret);
      setText(nextText);
      pendingCaret.current = atIndex;
      setCaret(atIndex);
      setActive(0);
      emit(nextText, picked);
      onTopicChange?.(item.topic);
      return;
    }
    const mention = item.mention!;
    const before = text.slice(0, atIndex);
    const insert = `@${mention.name} `;
    const nextText = before + insert + text.slice(caret);
    const nextPicked = picked.some((p) => p.personId === mention.personId)
      ? picked
      : [...picked, mention];
    setText(nextText);
    setPicked(nextPicked);
    pendingCaret.current = before.length + insert.length;
    setCaret(before.length + insert.length);
    setActive(0);
    emit(nextText, nextPicked);
  };

  const submit = () => onSubmit(serializeMentions(text, picked));

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    // Enter/Esc during IME composition belong to the IME.
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (e.key === 'Tab' && open) setDismissedAt(atIndex);
    if (e.key === 'Enter' && e.shiftKey && multiline) {
      e.preventDefault();
      return;
    }
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
      settled.current = true;
      submit();
    } else if (e.key === 'Escape' && onCancel) {
      e.preventDefault();
      settled.current = true;
      onCancel();
    }
  };

  let flatIndex = -1;
  return (
    <Popover open={open} onOpenChange={(o) => !o && setDismissedAt(atIndex)}>
      <PopoverAnchor asChild>
        {multiline ? (
        <AutoGrowTextarea
          ref={inputRef as never}
          value={text}
          autoFocus={autoFocus}
          placeholder={placeholder}
          aria-label={ariaLabel ?? placeholder}
          role="combobox"
          aria-autocomplete="list"
          aria-haspopup="listbox"
          aria-controls={listId}
          aria-expanded={open}
          aria-activedescendant={open ? activeOptionId : undefined}
          className={className}
          onChange={(e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
            settled.current = false;
            const v = multiline ? e.target.value.replace(/\n/g, ' ') : e.target.value;
            setText(v);
            setCaret(e.target.selectionStart ?? v.length);
            setActive(0);
            setDismissedAt(null);
            emit(v, picked);
          }}
          onFocus={() => {
            settled.current = false;
          }}
          onSelect={(e: SyntheticEvent<HTMLInputElement | HTMLTextAreaElement>) =>
            setCaret(e.currentTarget.selectionStart ?? e.currentTarget.value.length)
          }
          onKeyDown={onKeyDown}
          onBlur={() => {
            if (open) setDismissedAt(atIndex);
            if (submitOnBlur && !settled.current) {
              settled.current = true;
              submit();
            }
          }}
          rows={1}
          variant={variant}
          size={size}
        />
        ) : (
        <Input
          ref={inputRef as never}
          value={text}
          autoFocus={autoFocus}
          placeholder={placeholder}
          aria-label={ariaLabel ?? placeholder}
          role="combobox"
          aria-autocomplete="list"
          aria-haspopup="listbox"
          aria-controls={listId}
          aria-expanded={open}
          aria-activedescendant={open ? activeOptionId : undefined}
          className={className}
          variant={variant}
          onChange={(e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
            settled.current = false;
            const v = multiline ? e.target.value.replace(/\n/g, ' ') : e.target.value;
            setText(v);
            setCaret(e.target.selectionStart ?? v.length);
            setActive(0);
            setDismissedAt(null);
            emit(v, picked);
          }}
          onFocus={() => {
            settled.current = false;
          }}
          onSelect={(e: SyntheticEvent<HTMLInputElement | HTMLTextAreaElement>) =>
            setCaret(e.currentTarget.selectionStart ?? e.currentTarget.value.length)
          }
          onKeyDown={onKeyDown}
          onBlur={() => {
            if (open) setDismissedAt(atIndex);
            if (submitOnBlur && !settled.current) {
              settled.current = true;
              submit();
            }
          }}
        />
        )}
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-64 p-0"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
        // Keep focus in the field so picking with the mouse doesn't blur (and submit) it.
        onMouseDown={(e) => e.preventDefault()}
      >
        <Command
          shouldFilter={false}
          value={`mention-${activeIndex}`}
          onValueChange={(v) => {
            const n = Number(v.replace('mention-', ''));
            if (Number.isInteger(n)) setActive(n);
          }}
        >
          <CommandList ref={(el) => { listRef.current = el; if (el && el.id !== listId) setListId(el.id); }}>
            {groups.map((group) => (
              <CommandGroup key={group.heading} heading={group.heading}>
                {group.items.map((item) => {
                  const idx = ++flatIndex;
                  return (
                    <CommandItem key={item.key} value={`mention-${idx}`} onSelect={() => pick(item)}>
                      {item.label}
                      {item.hint && (
                        <span className="ml-auto pl-2 text-caption text-muted-foreground">{item.hint}</span>
                      )}
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

/** Textarea that grows with its content, so a long title wraps instead of scrolling. */
const AutoGrowTextarea = forwardRef<HTMLTextAreaElement, ComponentProps<typeof Textarea>>(
  function AutoGrowTextarea(props, ref) {
    const local = useRef<HTMLTextAreaElement | null>(null);
    const grow = () => {
      const el = local.current;
      if (!el) return;
      el.style.height = 'auto';
      el.style.height = `${el.scrollHeight}px`;
    };
    // Re-measure on every text change and whenever the column width changes (a wrap point
    // moves with the width, not with the value).
    useLayoutEffect(grow, [props.value]);
    useLayoutEffect(() => {
      const el = local.current;
      if (!el || typeof ResizeObserver === 'undefined') return;
      let width = el.clientWidth;
      const ro = new ResizeObserver(() => {
        if (el.clientWidth === width) return;
        width = el.clientWidth;
        grow();
      });
      ro.observe(el);
      return () => ro.disconnect();
    }, []);
    return (
      <Textarea
        {...props}
        ref={(el) => {
          local.current = el;
          if (typeof ref === 'function') ref(el);
          else if (ref) ref.current = el;
        }}
      />
    );
  },
);
