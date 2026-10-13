// SOT: chat-mention-input, at-mention-popover, composer-textarea
import { useEffect, useMemo, useRef, useState, type ChangeEvent, type KeyboardEvent, type SyntheticEvent } from "react";
import type { ChatMention, MentionKind } from "@/lib/bindings";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { PromptInputTextarea } from "@/components/ui/prompt-input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  MENTION_KIND_LABEL,
  activeMention,
  applyMention,
  filterCandidates,
  mentionOf,
  mentionsInText,
  type MentionCandidate,
} from "./mentions";

// WHAT:  The chat's message box: a textarea where typing `@` opens a list of the
//        tables, saved queries, dashboards, workflows and diagrams you can point at.
// WHY:   "Write a query for @orders" gives the model the real table instead of a
//        guess at what "orders" means, and "change @Sales" tells it which
//        dashboard to update. A list you can filter and pick from beats
//        remembering exact names.
// HOW:   The list is a plain listbox above the box, not a Radix popover, so focus
//        never leaves the textarea (a popover would take it). Arrow keys move,
//        Enter or Tab picks, Escape closes. Picking writes `@label ` into the text
//        and records the mention; a mention whose token is deleted from the text
//        is dropped when the message is sent.
// WHERE: src/features/chat/ChatTab.tsx, src/features/chat/mentions.ts

const KIND_ICON: Record<MentionKind, IconName> = {
  table: "table",
  saved_query: "code",
  dashboard: "chart-bar",
  workflow: "flow",
  diagram: "chart-relationship",
};

const KIND_TINT: Record<MentionKind, string> = {
  table: "text-success",
  saved_query: "text-accent",
  dashboard: "text-warning",
  workflow: "text-link",
  diagram: "text-danger",
};

interface MentionInputProps {
  value: string;
  onChange: (value: string) => void;
  mentions: readonly ChatMention[];
  onMentionsChange: (mentions: ChatMention[]) => void;
  candidates: readonly MentionCandidate[];
  placeholder: string;
  disabled: boolean;
}

export function MentionInput({ value, onChange, mentions, onMentionsChange, candidates, placeholder, disabled }: MentionInputProps) {
  const field = useRef<HTMLTextAreaElement>(null);
  const [caret, setCaret] = useState(0);
  const [index, setIndex] = useState(0);
  /// The `@` position the user closed the list at; it stays closed for that token.
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  /// Where the caret goes once React has written the inserted token.
  const pendingCaret = useRef<number | null>(null);

  const active = useMemo(() => activeMention(value, caret), [value, caret]);
  const items = useMemo(
    () => (active === null || dismissedAt === active.start ? [] : filterCandidates(candidates, active.query)),
    [active, candidates, dismissedAt],
  );
  const open = items.length > 0;
  const selected = Math.min(index, Math.max(items.length - 1, 0));

  // Puts the caret after an inserted token once React has written the new text.
  useEffect(() => {
    const at = pendingCaret.current;
    const node = field.current;
    if (at === null || node === null) return;
    pendingCaret.current = null;
    node.focus();
    node.setSelectionRange(at, at);
  }, [value]);

  const track = (event: SyntheticEvent<HTMLTextAreaElement>) => {
    setCaret(event.currentTarget.selectionStart);
  };

  const change = (event: ChangeEvent<HTMLTextAreaElement>) => {
    const next = event.target.value;
    onChange(next);
    setCaret(event.target.selectionStart);
    setIndex(0);
    // A token that is no longer in the text is no longer a mention.
    const kept = mentionsInText(next, mentions);
    if (kept.length !== mentions.length) onMentionsChange(kept);
  };

  const choose = (candidate: MentionCandidate) => {
    if (active === null) return;
    const applied = applyMention(value, active, candidate);
    onChange(applied.text);
    setCaret(applied.caret);
    pendingCaret.current = applied.caret;
    setIndex(0);
    if (!mentions.some((m) => m.kind === candidate.kind && m.id === candidate.id)) {
      onMentionsChange([...mentions, mentionOf(candidate)]);
    }
  };

  const keys = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!open) return;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setIndex((selected + 1) % items.length);
        break;
      case "ArrowUp":
        event.preventDefault();
        setIndex((selected - 1 + items.length) % items.length);
        break;
      case "Enter":
      case "Tab": {
        const pick = items[selected];
        if (pick !== undefined && !event.nativeEvent.isComposing) {
          event.preventDefault();
          choose(pick);
        }
        break;
      }
      case "Escape":
        event.preventDefault();
        if (active !== null) setDismissedAt(active.start);
        break;
      default:
        break;
    }
  };

  return (
    <div className="relative flex min-w-0 flex-1 flex-col">
      {open ? (
        <div
          role="listbox"
          aria-label="Mention a table, query or dashboard"
          className="absolute right-0 bottom-full left-0 z-30 mb-1.5 overflow-hidden rounded-lg glass-modal"
        >
          <ScrollArea className="max-h-64 p-0.5">
          {items.map((item, position) => {
            const firstOfKind = position === 0 || items[position - 1]?.kind !== item.kind;
            return (
              <div key={`${item.kind}:${item.id}`}>
                {firstOfKind ? (
                  <p className="px-2 pt-1 pb-0.5 text-[10px] font-semibold tracking-wider text-muted uppercase">
                    {MENTION_KIND_LABEL[item.kind]}
                  </p>
                ) : null}
                <button
                  type="button"
                  role="option"
                  aria-selected={position === selected}
                  // Mouse-down, not click: the textarea must keep focus.
                  onMouseDown={(event) => {
                    event.preventDefault();
                    choose(item);
                  }}
                  onMouseEnter={() => { setIndex(position); }}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs",
                    position === selected ? "bg-surface-secondary text-foreground" : "text-muted",
                  )}
                >
                  <Icon name={KIND_ICON[item.kind]} size={12} className={KIND_TINT[item.kind]} />
                  <span className="min-w-0 truncate font-mono text-[11px] text-foreground">{item.label}</span>
                  {item.detail !== null ? <span className="min-w-0 flex-1 truncate text-[10.5px] text-muted">{item.detail}</span> : null}
                </button>
              </div>
            );
          })}
          </ScrollArea>
        </div>
      ) : null}
      <PromptInputTextarea
        ref={field}
        value={value}
        onChange={change}
        onKeyDown={keys}
        onKeyUp={track}
        onClick={track}
        onSelect={track}
        // While the list is open Enter picks; it must not also send the message.
        submitOnEnter={!open}
        disabled={disabled}
        aria-label="Message"
        placeholder={placeholder}
      />
    </div>
  );
}
