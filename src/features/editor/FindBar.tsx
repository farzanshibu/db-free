// SOT: find-bar, find-replace-panel
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "@/lib/icons";
import { formatCount } from "@/lib/format";
import { IconButton } from "@/components/global/Button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Toggle } from "@/components/ui/toggle";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { MAX_MATCHES, type FindQuery, type FindStatus } from "./findReplace";

interface FindBarProps {
  /// What was selected when the bar opened; becomes the search text.
  initialSearch: string;
  /// Open on the replace row too.
  replaceOpen: boolean;
  status: FindStatus;
  onQuery: (query: FindQuery) => void;
  onNext: () => void;
  onPrevious: () => void;
  onReplace: () => void;
  onReplaceAll: () => void;
  onClose: () => void;
}

// WHAT:  The find / replace panel that floats over the top-right of the editor:
//        a search box with Aa / whole word / regex toggles, "3 of 12", previous
//        and next, and a replace row.
// WHY:   The editor is plain CodeMirror; the panel is the app's own components so
//        it matches every other surface and follows the theme.
// HOW:   Owns only what the user typed. Every change is reported as one
//        `FindQuery`; the editor answers with `status` (how many matches, which
//        one is current, whether the pattern is invalid).
// WHERE: src/features/editor/SqlEditor.tsx, src/features/editor/findReplace.ts
export function FindBar({ initialSearch, replaceOpen, status, onQuery, onNext, onPrevious, onReplace, onReplaceAll, onClose }: FindBarProps) {
  const [search, setSearch] = useState(initialSearch);
  const [replace, setReplace] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [regex, setRegex] = useState(false);
  const [showReplace, setShowReplace] = useState(replaceOpen);
  const searchRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    searchRef.current?.focus();
    searchRef.current?.select();
  }, []);

  // Every edit of the query, in one place.
  useEffect(() => {
    onQuery({ search, replace, caseSensitive, wholeWord, regex });
  }, [search, replace, caseSensitive, wholeWord, regex, onQuery]);

  const hasQuery = search.length > 0;
  const summary = status.error !== null ? "Invalid" : !hasQuery ? "" : status.count === 0 ? "No results" : `${status.current + 1} of ${formatCount(status.count)}${status.capped ? "+" : ""}`;
  const limit = `Stops at ${formatCount(MAX_MATCHES)} matches`;

  return (
    <div
      role="search"
      aria-label="Find in query"
      className="glass-modal absolute right-3 top-2 z-30 flex flex-col gap-1 rounded-lg p-1.5 shadow-lg"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="flex items-center gap-1">
        <Button variant="ghost" size="icon-sm" aria-label={showReplace ? "Hide replace" : "Show replace"} aria-expanded={showReplace} onClick={() => { setShowReplace((v) => !v); }}>
          <Icon name={showReplace ? "chevron-down" : "chevron-right"} size={12} />
        </Button>
        <Input
          ref={searchRef}
          aria-label="Find"
          aria-invalid={status.error !== null}
          placeholder="Find"
          value={search}
          spellCheck={false}
          className="h-6 w-52 font-mono text-xs"
          onChange={(event) => { setSearch(event.target.value); }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            if (event.shiftKey) onPrevious();
            else onNext();
          }}
        />
        <span className="w-16 shrink-0 text-center text-[11px] tabular-nums text-muted" title={status.error ?? (status.capped ? limit : "")}>
          {summary}
        </span>
        <OptionToggle label="Match case" pressed={caseSensitive} onChange={setCaseSensitive}>
          Aa
        </OptionToggle>
        <OptionToggle label="Whole word" pressed={wholeWord} onChange={setWholeWord}>
          <span className="underline decoration-1 underline-offset-2">ab</span>
        </OptionToggle>
        <OptionToggle label="Regular expression" pressed={regex} onChange={setRegex}>
          .*
        </OptionToggle>
        <IconButton icon="arrow-up" label="Previous match" onClick={onPrevious} disabled={status.count === 0} size={13} />
        <IconButton icon="arrow-down" label="Next match" onClick={onNext} disabled={status.count === 0} size={13} />
        <IconButton icon="x" label="Close" onClick={onClose} size={13} />
      </div>
      {showReplace ? (
        <div className="flex items-center gap-1 pl-5">
          <Input
            aria-label="Replace with"
            placeholder="Replace"
            value={replace}
            spellCheck={false}
            className="h-6 w-52 font-mono text-xs"
            onChange={(event) => { setReplace(event.target.value); }}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              onReplace();
            }}
          />
          <Button size="xs" variant="tertiary" disabled={status.count === 0} onClick={onReplace}>
            Replace
          </Button>
          <Button size="xs" variant="tertiary" disabled={status.count === 0} onClick={onReplaceAll}>
            All
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function OptionToggle({ label, pressed, onChange, children }: { label: string; pressed: boolean; onChange: (pressed: boolean) => void; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Toggle aria-label={label} pressed={pressed} onPressedChange={onChange} className="min-w-6 px-1.5 font-mono text-[11px]">
          {children}
        </Toggle>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
