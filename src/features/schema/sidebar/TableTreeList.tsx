// SOT: table-tree-list, virtualized-sidebar-list, tree-keyboard-navigation, tree-focus-management
import { useEffect, useMemo, useState, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { DropPlacement } from "../pointerDrag";
import { keyIntent, rowHeight, type KeyIntent, type TreeRow } from "@/lib/tableTree";
import { TreeRowView, type RowHandlers } from "./TreeRowView";

// ============================================================================
// WHAT:  The scrolling, virtualized, keyboard-driven list of sidebar rows.
// WHY:   A schema can hold thousands of tables; mounting a row for each makes the
//        sidebar the slowest part of the app. Only the rows in (and just around)
//        the viewport are rendered, at a fixed height per kind, so scroll cost
//        does not grow with the catalog.
// HOW:   The list is one focusable `role="tree"`; the row the keyboard is on is
//        announced through `aria-activedescendant` (rows come and go as they
//        scroll, so real DOM focus cannot live on them). Key presses go through
//        the pure `keyIntent` and come back out as intents for the panel to apply.
//        The focused row is scrolled into view through the virtualizer.
// WHERE: src/lib/tableTree.ts (rows, keyIntent), src/features/shell/TablesPanel.tsx
// ============================================================================

export interface HoverDrop {
  id: string;
  placement: DropPlacement;
}

interface TableTreeListProps {
  rows: readonly TreeRow[];
  /// Key of the row the keyboard is on.
  focusKey: string | null;
  /// `tableKey` of the table open in the active tab.
  activeRef: string | null;
  /// A tag reorder drag is over this header.
  hoverDrop: HoverDrop | null;
  handlers: RowHandlers;
  onIntent: (intent: KeyIntent) => void;
  /// Shift+F10 / the Menu key on the focused row.
  onMenuKey: (row: TreeRow, element: HTMLElement | null) => void;
  /// The tree element, so the search box can hand focus to it.
  treeRef: RefObject<HTMLDivElement | null>;
  /// The scrolling element, for drag auto-scroll.
  scrollRef: RefObject<HTMLDivElement | null>;
}

/// DOM ids cannot hold spaces and `aria-activedescendant` is a single token.
function domId(key: string): string {
  return `tt-${encodeURIComponent(key)}`;
}

export function TableTreeList({ rows, focusKey, activeRef, hoverDrop, handlers, onIntent, onMenuKey, treeRef, scrollRef }: TableTreeListProps) {
  const [treeFocused, setTreeFocused] = useState(false);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const row = rows[index];
      return row ? rowHeight(row) : 32;
    },
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 14,
  });

  // Row heights follow the row kind at an index, and a new sort order can put a
  // different kind at the same index with the same count.
  useEffect(() => {
    virtualizer.measure();
  }, [rows, virtualizer]);

  const focusIndex = useMemo(() => (focusKey === null ? -1 : rows.findIndex((r) => r.key === focusKey)), [rows, focusKey]);
  useEffect(() => {
    if (focusIndex >= 0) virtualizer.scrollToIndex(focusIndex, { align: "auto" });
  }, [focusIndex, virtualizer]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      const row = focusIndex >= 0 ? rows[focusIndex] : undefined;
      if (row) {
        event.preventDefault();
        onMenuKey(row, document.getElementById(domId(row.key)));
      }
      return;
    }
    const intent = keyIntent(rows, focusKey, event);
    if (intent.type === "none") return;
    event.preventDefault();
    onIntent(intent);
  };

  const focusedRow = focusIndex >= 0 ? rows[focusIndex] : undefined;

  return (
    <div
      ref={scrollRef}
      className="min-h-0 flex-1 overflow-y-auto px-1.5 py-1 [scrollbar-color:var(--color-surface-tertiary)_transparent] [scrollbar-width:thin]"
    >
      <div
        ref={treeRef}
        role="tree"
        aria-label="Tables"
        tabIndex={0}
        aria-activedescendant={focusedRow ? domId(focusedRow.key) : undefined}
        onKeyDown={onKeyDown}
        onFocus={() => { setTreeFocused(true); }}
        onBlur={() => { setTreeFocused(false); }}
        className="relative w-full rounded-lg outline-none"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <TreeRowView
              key={row.key}
              row={row}
              top={item.start}
              height={item.size}
              domId={domId(row.key)}
              focused={treeFocused && row.key === focusKey}
              active={row.kind === "table" && row.ref === activeRef}
              dropHint={row.kind === "group" && row.tag !== null && hoverDrop?.id === row.tag.id ? hoverDrop.placement : null}
              handlers={handlers}
            />
          );
        })}
      </div>
    </div>
  );
}
