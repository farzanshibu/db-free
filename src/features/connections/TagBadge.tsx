// SOT: tag-badge, tag-list, tag-overflow-expander
import type { ConnectionTag } from "@/lib/bindings";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { tagChipStyle } from "./connectionTags";

interface TagBadgeProps {
  tag: ConnectionTag;
  /// Makes the pill a button (filter by this tag).
  onSelect?: () => void;
  /// Shows a remove cross (the form's chip list).
  onRemove?: () => void;
  className?: string;
}

// WHAT:  One tag as a pill: name in the tag's colour on a tint of it.
// WHY:   The colour comes from a design token through `tagChipStyle`, so the pill
//        follows the light / dark theme instead of carrying a fixed hex.
export function TagBadge({ tag, onSelect, onRemove, className }: TagBadgeProps) {
  const pill = cn("inline-flex h-5 max-w-[9rem] shrink-0 items-center gap-1 rounded-full border px-2 text-[10.5px] font-medium leading-none", className);
  const label = <span className="truncate">{tag.name}</span>;
  return (
    <span className="inline-flex items-center">
      {onSelect ? (
        <Button variant="ghost" size="xs" onClick={onSelect} style={tagChipStyle(tag.color)} className={cn(pill, "h-5 min-w-0 rounded-full px-2 hover:brightness-125", onRemove ? "rounded-r-none border-r-0 pr-1.5" : "")} title={`Filter by ${tag.name}`}>
          {label}
        </Button>
      ) : (
        <span style={tagChipStyle(tag.color)} className={cn(pill, onRemove ? "rounded-r-none border-r-0 pr-1.5" : "")}>
          {label}
        </span>
      )}
      {onRemove ? (
        <Button variant="ghost" size="xs" aria-label={`Remove ${tag.name}`} onClick={onRemove} style={tagChipStyle(tag.color)} className="h-5 min-w-0 rounded-full rounded-l-none border border-l-0 px-1.5 hover:brightness-125">
          <Icon name="x" size={10} />
        </Button>
      ) : null}
    </span>
  );
}

interface TagListProps {
  tags: readonly ConnectionTag[];
  /// How many pills show before the rest fold into a "+N" button.
  max?: number;
  onSelect?: (tag: ConnectionTag) => void;
}

// WHAT:  A connection's tags in a row, the overflow folded into "+N".
// WHY:   A card is a fixed height; five tags would push the host line away.
//        "+N" opens the remaining pills in a popover rather than hiding them.
export function TagList({ tags, max = 3, onSelect }: TagListProps) {
  if (tags.length === 0) return null;
  const shown = tags.slice(0, max);
  const rest = tags.slice(max);
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-1">
      {shown.map((tag) => (
        <TagBadge key={tag.id} tag={tag} {...(onSelect ? { onSelect: () => { onSelect(tag); } } : {})} />
      ))}
      {rest.length > 0 ? (
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="xs" aria-label={`${rest.length} more tags`} className="h-5 min-w-0 rounded-full border border-border px-1.5 text-[10.5px] text-muted">
              +{rest.length}
            </Button>
          </PopoverTrigger>
          <PopoverContent className="flex w-auto max-w-64 flex-wrap gap-1 p-2">
            {rest.map((tag) => (
              <TagBadge key={tag.id} tag={tag} {...(onSelect ? { onSelect: () => { onSelect(tag); } } : {})} />
            ))}
          </PopoverContent>
        </Popover>
      ) : null}
    </span>
  );
}
