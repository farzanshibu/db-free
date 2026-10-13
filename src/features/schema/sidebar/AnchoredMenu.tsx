// SOT: anchored-menu, nested-menu-nodes, checkable-menu-items, menu-keyboard-open
import { useCallback, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { ConnectionColor } from "@/lib/bindings";
import { connectionColorMeta } from "@/lib/connectionGroups";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";

// ============================================================================
// WHAT:  A pointer-anchored menu that, unlike the shared flat `useContextMenu`,
//        has submenus and checkable entries — the sidebar's "Tag table" submenu
//        needs both.
// WHY:   The shared context menu is a flat list of actions (and is used by many
//        screens, so it stays that). A table's menu is a tree: Tag table ▸ with a
//        checkmark per tag you can toggle one after another without the menu
//        closing.
// HOW:   Same technique as the shared one: an invisible fixed trigger portalled to
//        <body> at the pointer (a mask on a scrolling ancestor would otherwise
//        move a `position: fixed` anchor), the shadcn DropdownMenu on top. The
//        entries come from a `build` function re-run on every render, so a
//        checkmark flipped by an optimistic store update shows up while the
//        menu is still open. Menus are described as data (`MenuNode`) so they can
//        be built by pure functions (`tableMenus.ts`).
// WHERE: src/features/schema/sidebar/tableMenus.ts, src/features/shell/TablesPanel.tsx
// ============================================================================

export type MenuNode =
  | {
      kind: "item";
      id: string;
      label: string;
      icon?: IconName;
      danger?: boolean;
      disabled?: boolean;
      /// Muted second line: why the entry is unavailable, or what to know first.
      note?: string;
      /// Right-aligned gesture hint (`Shift+Click`).
      hint?: string;
    }
  /// A toggle that leaves the menu open, so several can be flipped in a row.
  | { kind: "check"; id: string; label: string; checked: boolean; color?: ConnectionColor; disabled?: boolean }
  | { kind: "sub"; label: string; icon?: IconName; disabled?: boolean; note?: string; children: readonly MenuNode[] }
  | { kind: "separator" };

/// Drops separators that would sit first, last or twice in a row.
function tidy(nodes: readonly MenuNode[]): MenuNode[] {
  const out: MenuNode[] = [];
  for (const node of nodes) {
    if (node.kind === "separator" && (out.length === 0 || out[out.length - 1]?.kind === "separator")) continue;
    out.push(node);
  }
  while (out[out.length - 1]?.kind === "separator") out.pop();
  return out;
}

function Label({ label, note }: { label: string; note?: string | undefined }) {
  if (note === undefined) return <span className="truncate">{label}</span>;
  return (
    <span className="flex min-w-0 flex-col">
      <span className="truncate">{label}</span>
      <span className="whitespace-normal text-[11px] leading-snug text-muted">{note}</span>
    </span>
  );
}

// WHAT:  Renders menu nodes inside any `DropdownMenuContent` / `DropdownMenuSubContent`.
export function MenuNodes({ nodes, onSelect }: { nodes: readonly MenuNode[]; onSelect: (id: string) => void }): ReactNode {
  return tidy(nodes).map((node, index) => {
    switch (node.kind) {
      case "separator":
        return <DropdownMenuSeparator key={`sep-${String(index)}`} />;
      case "item":
        return (
          <DropdownMenuItem key={node.id} disabled={node.disabled ?? false} variant={node.danger === true ? "danger" : "default"} onSelect={() => { onSelect(node.id); }}>
            {node.icon !== undefined ? <Icon name={node.icon} size={13} className={cn("shrink-0", node.danger === true ? "text-danger" : "text-muted")} /> : null}
            <Label label={node.label} note={node.note} />
            {node.hint !== undefined ? <DropdownMenuShortcut>{node.hint}</DropdownMenuShortcut> : null}
          </DropdownMenuItem>
        );
      case "check":
        return (
          <DropdownMenuCheckboxItem
            key={node.id}
            checked={node.checked}
            disabled={node.disabled ?? false}
            onSelect={(event) => {
              event.preventDefault();
              onSelect(node.id);
            }}
          >
            {node.color !== undefined ? <span aria-hidden className={cn("size-2 shrink-0 rounded-full", connectionColorMeta(node.color).fill)} /> : null}
            <span className="truncate">{node.label}</span>
          </DropdownMenuCheckboxItem>
        );
      case "sub":
        return (
          <DropdownMenuSub key={`sub-${node.label}`}>
            <DropdownMenuSubTrigger disabled={node.disabled ?? false}>
              {node.icon !== undefined ? <Icon name={node.icon} size={13} className="shrink-0 text-muted" /> : null}
              <Label label={node.label} note={node.note} />
            </DropdownMenuSubTrigger>
            <DropdownMenuPortal>
              <DropdownMenuSubContent className="min-w-44">
                <ScrollArea className="max-h-72">
                  <MenuNodes nodes={node.children} onSelect={onSelect} />
                </ScrollArea>
              </DropdownMenuSubContent>
            </DropdownMenuPortal>
          </DropdownMenuSub>
        );
    }
  });
}

interface MenuState {
  x: number;
  y: number;
  build: () => readonly MenuNode[];
  onSelect: (id: string) => void;
}

export interface AnchoredMenu {
  /// Opens at a viewport position.
  openAt: (x: number, y: number, build: () => readonly MenuNode[], onSelect: (id: string) => void) => void;
  /// Opens at the pointer of a right-click.
  openFromEvent: (event: ReactMouseEvent, build: () => readonly MenuNode[], onSelect: (id: string) => void) => void;
  /// Opens under an element (a "⋯" button, or the focused row for Shift+F10).
  openBelow: (element: Element, build: () => readonly MenuNode[], onSelect: (id: string) => void) => void;
  /// Render once inside the component that owns the menu.
  node: ReactNode;
}

export function useAnchoredMenu(): AnchoredMenu {
  const [state, setState] = useState<MenuState | null>(null);

  const openAt = useCallback((x: number, y: number, build: () => readonly MenuNode[], onSelect: (id: string) => void) => {
    setState({ x, y, build, onSelect });
  }, []);
  const openFromEvent = useCallback(
    (event: ReactMouseEvent, build: () => readonly MenuNode[], onSelect: (id: string) => void) => {
      event.preventDefault();
      event.stopPropagation();
      openAt(event.clientX, event.clientY, build, onSelect);
    },
    [openAt],
  );
  const openBelow = useCallback(
    (element: Element, build: () => readonly MenuNode[], onSelect: (id: string) => void) => {
      const box = element.getBoundingClientRect();
      openAt(box.left, box.bottom, build, onSelect);
    },
    [openAt],
  );

  const node =
    state === null
      ? null
      : createPortal(
          <DropdownMenu open onOpenChange={(next) => { if (!next) setState(null); }}>
            <DropdownMenuTrigger aria-label="Context menu" tabIndex={-1} className="pointer-events-none fixed z-50 size-px opacity-0" style={{ left: state.x, top: state.y }} />
            <DropdownMenuContent align="start" className="min-w-48">
              <MenuNodes nodes={state.build()} onSelect={state.onSelect} />
            </DropdownMenuContent>
          </DropdownMenu>,
          document.body,
        );

  return { openAt, openFromEvent, openBelow, node };
}
