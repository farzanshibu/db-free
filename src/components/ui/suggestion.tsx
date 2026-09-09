// SOT: suggestion, starter-prompts
import type { ComponentProps } from "react";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn AI Elements Suggestion: a tappable starter prompt.
// WHY:   An empty chat is the hardest moment to use it; naming a few things it
//        can actually do is faster than explaining the capability in prose.
// WHERE: https://ai-sdk.dev/elements/components/suggestion
export function Suggestions({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="suggestions" className={cn("grid w-full grid-cols-1 gap-2 sm:grid-cols-2", className)} {...props} />;
}

export interface SuggestionProps extends Omit<ComponentProps<"button">, "onClick" | "onSelect"> {
  suggestion: string;
  onSelect: (suggestion: string) => void;
}

export function Suggestion({ className, suggestion, onSelect, ...props }: SuggestionProps) {
  return (
    <button
      type="button"
      data-slot="suggestion"
      onClick={() => { onSelect(suggestion); }}
      className={cn(
        "flex items-center justify-between gap-2 rounded-xl border border-border/50 bg-surface-secondary/40 p-2.5",
        "text-left text-xs text-foreground transition-all liquid-hover",
        "hover:border-accent/40 hover:bg-surface-secondary/80 active:scale-[0.99]",
        className,
      )}
      {...props}
    >
      <span className="line-clamp-2">{suggestion}</span>
      <Icon name="arrow-up" size={10} className="shrink-0 rotate-45 text-muted opacity-60" />
    </button>
  );
}
