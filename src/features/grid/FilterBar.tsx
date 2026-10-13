// SOT: filter-bar, typed-filter-input, filter-autocomplete, filter-inline-errors
import { useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import type { FilterMode } from "@/lib/bindings";
import { formatFilterExpr, parseFilterExpr, suggestFilter, type Suggestion } from "@/lib/filterExpr";
import { displayChord } from "@/lib/keymap";
import { useKeymap } from "@/stores/useShortcut";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { activeRules, opsForType, type FilterEntry } from "./filterOps";
import { ScrollArea } from "@/components/ui/scroll-area";

interface FilterBarProps {
  columns: readonly { name: string; typeName?: string | undefined }[];
  entries: readonly FilterEntry[];
  mode: FilterMode;
  /// Applies the typed filter. Rules switched off in the popover are kept as they were.
  onApply: (entries: FilterEntry[], mode: FilterMode) => void;
  inputRef: RefObject<HTMLInputElement | null>;
}

// WHAT:  The typed filter: `status = active and revenue > 100`, with completion for
//        columns and operators and an inline message when it does not parse.
// WHY:   For someone who knows the question, typing it beats opening a popover and
//        picking a column, an operator and a value three times over. The popover
//        and this bar edit the same rules, so either can finish what the other began.
// HOW:   The text shown is the canonical form of the rules in force until the user
//        types (`draft`); Enter applies, Escape drops the draft, Tab takes the
//        highlighted completion. An error at the very end of the text ("expected a
//        value") is only a hint until Enter is pressed; one earlier is shown at once.
// WHERE: src/lib/filterExpr.ts (grammar), src/features/grid/TableTab.tsx
export function FilterBar({ columns, entries, mode, onApply, inputRef }: FilterBarProps) {
  const canonical = useMemo(() => formatFilterExpr(activeRules(entries), mode), [entries, mode]);
  const [draft, setDraft] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [focused, setFocused] = useState(false);
  const [picked, setPicked] = useState(-1);
  const [submitted, setSubmitted] = useState(false);
  const wrapper = useRef<HTMLDivElement | null>(null);
  const text = draft ?? canonical;
  // The chord that focuses this bar, as the user has it bound.
  const bound = useKeymap()["focus-filter"];
  const chord = bound.length > 0 ? displayChord(bound) : "";

  const parsed = useMemo(() => parseFilterExpr(text, columns), [text, columns]);
  const suggestions = useMemo(() => (focused ? suggestFilter(text, cursor, columns, opsForType) : { from: cursor, to: cursor, options: [] }), [focused, text, cursor, columns]);
  const error = !parsed.ok ? parsed.errors[0] : undefined;
  // "Expected a value" at the end of the text is a hint while typing, not a mistake.
  const incomplete = error !== undefined && error.start >= text.trim().length;
  const showError = error !== undefined && (!incomplete || submitted);
  const dirty = draft !== null && draft !== canonical;

  const apply = () => {
    setSubmitted(true);
    if (!parsed.ok) return;
    const off = entries.filter((e) => !e.enabled);
    onApply([...parsed.rules.map((rule) => ({ rule, enabled: true })), ...off], parsed.mode);
    setDraft(null);
    setSubmitted(false);
  };
  const accept = (option: Suggestion) => {
    const next = `${text.slice(0, suggestions.from)}${option.insert}${text.slice(suggestions.to)}`;
    setDraft(next);
    setPicked(-1);
    const at = suggestions.from + option.insert.length;
    requestAnimationFrame(() => {
      inputRef.current?.setSelectionRange(at, at);
      setCursor(at);
    });
  };
  const onKey = (event: KeyboardEvent<HTMLInputElement>) => {
    const options = suggestions.options;
    if (event.key === "ArrowDown" && options.length > 0) {
      event.preventDefault();
      setPicked((i) => (i + 1) % options.length);
    } else if (event.key === "ArrowUp" && options.length > 0) {
      event.preventDefault();
      setPicked((i) => (i <= 0 ? options.length - 1 : i - 1));
    } else if (event.key === "Tab" && options.length > 0 && !event.shiftKey) {
      event.preventDefault();
      const option = options[Math.max(0, picked)];
      if (option) accept(option);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const option = picked >= 0 ? options[picked] : undefined;
      if (option) accept(option);
      else apply();
    } else if (event.key === "Escape") {
      event.preventDefault();
      setDraft(null);
      setSubmitted(false);
      setPicked(-1);
      inputRef.current?.blur();
    }
  };

  return (
    <div ref={wrapper} className="relative flex h-7 shrink-0 items-center gap-1.5 border-b border-border/40 px-2">
      <Icon name="filter" size={12} className={cn("shrink-0", entries.some((e) => e.enabled) ? "text-accent" : "text-muted")} />
      <input
        ref={inputRef}
        value={text}
        spellCheck={false}
        autoComplete="off"
        placeholder={`Filter rows — status = active and revenue > 100${chord.length > 0 ? `   (${chord})` : ""}`}
        aria-label="Filter rows"
        aria-invalid={showError}
        onChange={(e) => {
          setDraft(e.target.value);
          setCursor(e.target.selectionStart ?? e.target.value.length);
          setPicked(-1);
          setSubmitted(false);
        }}
        onKeyDown={onKey}
        onKeyUp={(e) => setCursor(e.currentTarget.selectionStart ?? e.currentTarget.value.length)}
        onClick={(e) => setCursor(e.currentTarget.selectionStart ?? e.currentTarget.value.length)}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          setPicked(-1);
        }}
        className={cn(
          "h-5.5 min-w-0 flex-1 rounded-md border bg-transparent px-2 font-mono text-xs text-foreground outline-none placeholder:text-muted/70",
          showError ? "border-danger" : "border-transparent focus:border-accent hover:border-border/60",
        )}
      />
      {error !== undefined && (showError || (incomplete && focused)) ? (
        <span className={cn("max-w-[40%] shrink-0 truncate text-[11px]", showError ? "text-danger" : "text-muted")} role={showError ? "alert" : undefined}>
          {error.message}
        </span>
      ) : null}
      {dirty ? (
        <Button size="xs" variant="soft" onClick={apply} disabled={!parsed.ok} aria-label="Apply filter">
          Apply
          <span className="ml-1 text-[10px] opacity-70">↵</span>
        </Button>
      ) : null}
      {text.length > 0 ? (
        <Button
          size="xs"
          variant="toolbar"
          aria-label="Clear filter"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            onApply(entries.filter((e) => !e.enabled), "all");
            setDraft(null);
            setSubmitted(false);
          }}
        >
          <Icon name="x" size={11} />
        </Button>
      ) : null}
      {focused && suggestions.options.length > 0 ? (
        <div className="absolute left-7 top-7 z-30 min-w-52 overflow-hidden rounded-lg border border-border/60 bg-surface shadow-lg">
        <ScrollArea orientation="both" className="max-h-60 p-1">
        <ul role="listbox" aria-label="Filter suggestions">
          {suggestions.options.slice(0, 40).map((option, index) => (
            <li key={`${option.kind}-${option.label}`} role="option" aria-selected={index === picked}>
              <button
                type="button"
                // Mouse-down, not click: the input must not lose focus (and the list with it) first.
                onMouseDown={(e) => {
                  e.preventDefault();
                  accept(option);
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded-md px-2 py-0.5 text-left font-mono text-xs",
                  index === picked ? "bg-accent/20 text-foreground" : "text-foreground hover:bg-surface-secondary/60",
                )}
              >
                <span className="truncate">{option.label}</span>
                {option.detail ? <span className="ml-auto truncate pl-2.5 text-[10px] text-muted">{option.detail}</span> : null}
                {option.kind === "operator" ? <span className="ml-auto text-[10px] uppercase text-muted">op</span> : null}
              </button>
            </li>
          ))}
        </ul>
        </ScrollArea>
        </div>
      ) : null}
    </div>
  );
}
