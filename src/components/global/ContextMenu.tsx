// SOT: context-menu, right-click-menu, menu-at-pointer
import { useCallback, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";

export interface MenuEntry {
  id: string;
  label: string;
  icon?: IconName;
  /// Rendered in the danger colour (close all, delete…).
  danger?: boolean;
  disabled?: boolean;
  /// Consecutive entries sharing a group render inside one separated section.
  group?: string;
}

interface MenuState {
  x: number;
  y: number;
  entries: readonly MenuEntry[];
  onAction: (id: string) => void;
}

export interface ContextMenu {
  /// Call from an `onContextMenu` handler: opens the menu at the pointer.
  open: (event: ReactMouseEvent, entries: readonly MenuEntry[], onAction: (id: string) => void) => void;
  /// Render once inside the component that owns the menu.
  node: ReactNode;
}

// WHAT:  Right-click menu anchored to the pointer, built on the shadcn
//        DropdownMenu so it inherits menu keyboard handling, focus return and
//        the glass popover styling.
// WHY:   Radix's own ContextMenu anchors to a Trigger element that has to wrap
//        the target. The data grid's cells are absolutely positioned inside a
//        virtualized track and cannot be wrapped, so a zero-size fixed anchor
//        placed at the click point gives the popover something to position
//        against without touching the caller's layout.
// HOW:   `open(event, entries, onAction)` stores the pointer position and the
//        entries; the menu closes on action, Escape or an outside press.
// WHERE: src/features/shell/TabBar.tsx, src/features/grid/DataGrid.tsx
export function useContextMenu(): ContextMenu {
  const [state, setState] = useState<MenuState | null>(null);

  const open = useCallback((event: ReactMouseEvent, entries: readonly MenuEntry[], onAction: (id: string) => void) => {
    if (entries.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    setState({ x: event.clientX, y: event.clientY, entries, onAction });
  }, []);

  // Portalled to <body>: a `position: fixed` anchor is positioned against the
  // nearest ancestor that has a mask, filter or transform — and the scroll
  // fades the grid and tab bar use are mask-image, which made the menu open
  // an entire container away from the pointer.
  const node =
    state === null
      ? null
      : createPortal(
          <DropdownMenu
            open
            onOpenChange={(next) => {
              if (!next) setState(null);
            }}
          >
            <DropdownMenuTrigger
              aria-label="Context menu"
              tabIndex={-1}
              className="pointer-events-none fixed z-50 size-px opacity-0"
              style={{ left: state.x, top: state.y }}
            />
            <DropdownMenuContent align="start" className="min-w-52">
              {groupEntries(state.entries).map((group, index) => (
                <DropdownMenuGroup key={group[0]?.group ?? String(index)}>
                  {index > 0 ? <DropdownMenuSeparator /> : null}
                  {group.map((entry) => (
                    <DropdownMenuItem
                      key={entry.id}
                      disabled={entry.disabled ?? false}
                      variant={entry.danger === true ? "danger" : "default"}
                      onSelect={() => {
                        state.onAction(entry.id);
                        setState(null);
                      }}
                    >
                      {entry.icon !== undefined ? <Icon name={entry.icon} size={13} className={cn("shrink-0", entry.danger === true ? "text-danger" : "text-muted")} /> : null}
                      <span className="truncate">{entry.label}</span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuGroup>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>,
          document.body,
        );

  return { open, node };
}

/// Splits the flat list into runs of the same `group`, so callers describe
/// sections by tagging entries instead of nesting arrays.
function groupEntries(entries: readonly MenuEntry[]): MenuEntry[][] {
  const groups: MenuEntry[][] = [];
  for (const entry of entries) {
    const last = groups[groups.length - 1];
    if (last && (last[0]?.group ?? "") === (entry.group ?? "")) last.push(entry);
    else groups.push([entry]);
  }
  return groups;
}
