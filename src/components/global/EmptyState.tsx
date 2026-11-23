// SOT: empty-state-component
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { InfoTip } from "./InfoTip";

interface EmptyStateProps {
  /// What is missing, in a few words ("No tab open", "Nothing pinned").
  title: string;
  /// At most one short sentence that is needed to act. Background goes in `tip`.
  body?: string | undefined;
  /// Background or "how it works", behind an info icon beside the title.
  tip?: ReactNode;
  /// The one action that fills the empty space (a Button), plus at most one secondary.
  action?: ReactNode;
  icon?: IconName | undefined;
  /// `page` fills a tab or page and centres; `inline` sits inside a panel or list.
  size?: "page" | "inline";
  className?: string | undefined;
}

// WHAT:  What a view shows when it has nothing to show: icon, title, the action
//        that fixes it.
// WHY:   An empty view is the first thing a new user sees in every feature; it
//        is a prompt to act, not a paragraph to read. The refresh rule is
//        icon + title + action, with the explanation behind a tip.
// WHERE: docs/ui-refresh.md ("Empty states")
export function EmptyState({ title, body, tip, action, icon, size = "page", className }: EmptyStateProps) {
  const inline = size === "inline";
  return (
    <div className={cn("flex flex-col items-center justify-center text-center", inline ? "gap-1.5 px-3 py-4" : "h-full gap-2 p-5", className)}>
      {icon ? (
        <span className={cn("mb-0.5 flex items-center justify-center rounded-xl glass-card text-accent", inline ? "size-7" : "size-9 shadow-lg")}>
          <Icon name={icon} size={inline ? 14 : 16} />
        </span>
      ) : null}
      <p className="flex items-center gap-1 text-[13px] font-medium tracking-tight text-foreground">
        {title}
        {tip !== undefined && tip !== null && tip !== "" ? <InfoTip label={`About: ${title}`}>{tip}</InfoTip> : null}
      </p>
      {body ? <p className="max-w-sm text-xs leading-relaxed text-muted">{body}</p> : null}
      {action ? <div className="mt-1 flex flex-wrap items-center justify-center gap-1.5">{action}</div> : null}
    </div>
  );
}
