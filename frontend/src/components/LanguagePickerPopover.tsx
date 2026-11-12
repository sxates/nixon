"use client";

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { DeckIcon } from "@/components/ui/deck-icon";
import { Check } from "@/components/ui/icons";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { LANGUAGE_OPTIONS } from "@/lib/summary-languages";
import { useRecentLanguages } from "@/hooks/useRecentLanguages";

/** One pick-list row: a full-width ghost Button, brand text + check when it is the choice. */
function LanguageOption({
  selected,
  onSelect,
  children,
}: {
  selected: boolean;
  onSelect: () => void;
  children: ReactNode;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "h-auto w-full justify-between rounded-none px-3 py-1.5 text-left font-normal hover:bg-muted",
        selected ? "font-medium text-brand hover:text-brand" : "text-foreground",
      )}
    >
      {children}
      {selected && <DeckIcon icon={Check} size={14} className="text-brand" />}
    </Button>
  );
}

interface LanguagePickerPopoverProps {
  value: string | null;
  onChange: (code: string | null) => void;
  onClose: () => void;
  mode?: "meeting" | "settings";
  autoSubtitle?: string;
}

export function LanguagePickerPopover({
  value,
  onChange,
  onClose,
  mode = "meeting",
  autoSubtitle,
}: LanguagePickerPopoverProps) {
  const { recents } = useRecentLanguages();
  const [query, setQuery] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (!containerRef.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const filter = query.trim().toLowerCase();

  const recentCodes = useMemo(() => new Set(recents), [recents]);

  const filteredAll = useMemo(() => {
    const options = mode === "meeting"
      ? LANGUAGE_OPTIONS.filter((l) => !recentCodes.has(l.code))
      : LANGUAGE_OPTIONS;
    if (!filter) return options;
    return options.filter(
      (l) =>
        l.code.toLowerCase().includes(filter) ||
        l.label.toLowerCase().includes(filter),
    );
  }, [filter, mode, recentCodes]);

  const recentsResolved = useMemo(
    () =>
      recents
        .map((code) => LANGUAGE_OPTIONS.find((l) => l.code === code))
        .filter((l): l is (typeof LANGUAGE_OPTIONS)[number] => Boolean(l))
        .filter(
          (l) =>
            !filter ||
            l.code.toLowerCase().includes(filter) ||
            l.label.toLowerCase().includes(filter),
        ),
    [recents, filter],
  );

  const showAuto = mode === "meeting" && (!filter || "auto".includes(filter));
  const showRecents = mode === "meeting" && recentsResolved.length > 0;
  const hasNoResults =
    filteredAll.length === 0 && recentsResolved.length === 0 && !showAuto;

  return (
    <div
      ref={containerRef}
      className="w-72 rounded-lg bg-popover text-popover-foreground border border-border shadow-lg overflow-hidden"
      role="dialog"
      aria-label="Pick summary language"
    >
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-border">
        <Input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search language..."
          aria-label="Search language"
          className="h-7 flex-1 border-none px-1 shadow-none"
        />
      </div>

      <div className="max-h-80 overflow-y-auto py-1">
        {showRecents && (
          <>
            <div className="u-section-label px-3 pb-1 pt-1">
              Recently Used
            </div>
            {recentsResolved.map((opt) => (
              <LanguageOption
                key={`recent-${opt.code}`}
                selected={value === opt.code}
                onSelect={() => onChange(opt.code)}
              >
                <span>
                  {opt.label}{" "}
                  <span className="text-xs text-muted-foreground">({opt.code})</span>
                </span>
              </LanguageOption>
            ))}
            <div className="my-1 h-px bg-border" />
          </>
        )}

        {showAuto && (
          <LanguageOption selected={value === null} onSelect={() => onChange(null)}>
            <span className="flex flex-col items-start">
              <span>Auto</span>
              {autoSubtitle && (
                <span className="text-xs font-normal text-muted-foreground">{autoSubtitle}</span>
              )}
            </span>
          </LanguageOption>
        )}

        {filteredAll.length > 0 && (
          <div className="u-section-label px-3 pb-1 pt-1">
            {mode === "meeting" ? "Other Languages" : "All Languages"}
          </div>
        )}

        {filteredAll.map((opt) => (
          <LanguageOption
            key={`all-${opt.code}`}
            selected={value === opt.code}
            onSelect={() => onChange(opt.code)}
          >
            <span>
              {opt.label}{" "}
              <span className="text-xs text-muted-foreground">({opt.code})</span>
            </span>
          </LanguageOption>
        ))}

        {hasNoResults && (
          <div className="px-3 py-2 text-sm text-muted-foreground">No matches</div>
        )}
      </div>
    </div>
  );
}
