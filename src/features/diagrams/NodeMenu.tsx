// SOT: node-context-menu, diagram-right-click-menu, table-colour-menu
import { createPortal } from "react-dom";
import type { ConnectionColor } from "@/lib/bindings";
import type { MenuEntry } from "@/components/global/ContextMenu";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { ColorSwatches } from "./ColorSwatches";

export interface NodeMenuProps {
  /// Where the pointer was, in viewport coordinates.
  at: { x: number; y: number };
  onClose: () => void;
  /// Heading: the table's name.
  title: string;
  /// Offers the colour swatches when present.
  color?: { value: ConnectionColor | null; onChange: (color: ConnectionColor | null) => void; resetLabel: string };
  entries: readonly MenuEntry[];
  onAction: (id: string) => void;
}

// WHAT:  The right-click menu of a table card: its colour, then its actions.
// WHY:   The shared `useContextMenu` takes a flat list of entries; a colour grid is
//        not an entry, so this menu is built on the same primitives (a DropdownMenu
//        anchored at the pointer by a zero-size trigger) with room for swatches.
// HOW:   Portalled to <body> so a masked or transformed ancestor cannot shift the
//        fixed-position anchor away from the pointer.
export function NodeMenu({ at, onClose, title, color, entries, onAction }: NodeMenuProps) {
  return createPortal(
    <DropdownMenu
      open
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DropdownMenuTrigger aria-label="Table menu" tabIndex={-1} className="pointer-events-none fixed z-50 size-px opacity-0" style={{ left: at.x, top: at.y }} />
      <DropdownMenuContent align="start" className="min-w-52">
        <DropdownMenuLabel className="truncate">{title}</DropdownMenuLabel>
        {color ? (
          <div className="px-2 pt-0.5 pb-1.5">
            <ColorSwatches value={color.value} onChange={color.onChange} resetLabel={color.resetLabel} />
          </div>
        ) : null}
        {color && entries.length > 0 ? <DropdownMenuSeparator /> : null}
        {entries.map((entry) => (
          <DropdownMenuItem
            key={entry.id}
            disabled={entry.disabled ?? false}
            variant={entry.danger === true ? "danger" : "default"}
            onSelect={() => {
              onAction(entry.id);
              onClose();
            }}
          >
            {entry.icon !== undefined ? <Icon name={entry.icon} size={13} className={cn("shrink-0", entry.danger === true ? "text-danger" : "text-muted")} /> : null}
            <span className="truncate">{entry.label}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>,
    document.body,
  );
}
