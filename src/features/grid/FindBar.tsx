// SOT: find-bar, grid-find-ui
import { useEffect, type KeyboardEvent, type RefObject } from "react";
import { cn } from "@/lib/cn";
import { IconButton } from "@/components/global/Button";
import { Input } from "@/components/ui/input";
import { Toggle } from "@/components/ui/toggle";
import { formatCount } from "@/lib/format";

interface FindBarProps {
  query: string;
  onQuery: (query: string) => void;
  matchCase: boolean;
  onMatchCase: (on: boolean) => void;
  /// Hits on the page, and which one is current (0-based).
  total: number;
  index: number;
  capped: boolean;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
  inputRef: RefObject<HTMLInputElement | null>;
}

// WHAT:  The find bar over the grid: query, match case, "3 of 12", previous / next.
// HOW:   Enter steps forward and Shift+Enter back, Escape closes. It opens focused
//        with the old query selected, so ⌘F ⌘F types over the last search.
// WHERE: src/features/grid/TableTab.tsx, src/features/grid/gridFind.ts
export function FindBar({ query, onQuery, matchCase, onMatchCase, total, index, capped, onNext, onPrev, onClose, inputRef }: FindBarProps) {
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [inputRef]);
  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      if (event.shiftKey) onPrev();
      else onNext();
    } else if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  };
  const status = query.length === 0 ? "" : total === 0 ? "No results" : `${formatCount(index + 1)} of ${formatCount(total)}${capped ? "+" : ""}`;
  return (
    <div role="search" className="absolute right-3 top-8 z-30 flex items-center gap-1 rounded-lg border border-border/60 bg-surface p-1 shadow-md">
      <Input
        ref={inputRef}
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        onKeyDown={onKey}
        placeholder="Find in this page"
        aria-label="Find in this page"
        className="h-6 w-44 min-w-0 rounded-md px-2 font-mono text-xs"
      />
      <Toggle pressed={matchCase} onPressedChange={onMatchCase} aria-label="Match case" className="h-6 min-w-6 px-1.5 font-mono text-[11px]">
        Aa
      </Toggle>
      <span className={cn("min-w-16 text-center font-mono text-[11px] tabular-nums", total === 0 && query.length > 0 ? "text-danger" : "text-muted")} aria-live="polite">
        {status}
      </span>
      <IconButton icon="arrow-up" label="Previous match" disabled={total === 0} onClick={onPrev} size={12} className="size-6 min-w-6" />
      <IconButton icon="arrow-down" label="Next match" disabled={total === 0} onClick={onNext} size={12} className="size-6 min-w-6" />
      <IconButton icon="x" label="Close find" onClick={onClose} size={12} className="size-6 min-w-6" />
    </div>
  );
}
