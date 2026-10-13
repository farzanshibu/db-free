// SOT: tree-row-view, sidebar-table-row, tag-group-header-row, column-preview-row, tag-colour-dots
import { memo, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { TableInfo, TableTag } from "@/lib/bindings";
import { connectionColorMeta } from "@/lib/connectionGroups";
import { formatCount } from "@/lib/format";
import { Icon, typeIcon } from "@/lib/icons";
import type { DropPlacement } from "../pointerDrag";
import { cn } from "@/lib/cn";
import type { TreeRow } from "@/lib/tableTree";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

// ============================================================================
// WHAT:  How each kind of sidebar row looks and which gestures it answers: schema
//        header, tag group header, table, the column preview under a table.
// WHY:   The list is virtualized, so a row is rendered by position, not owned by a
//        component with state. Every row therefore takes everything as props and
//        is memoised: scrolling or focusing one row re-renders that row, not 5 000.
// HOW:   `handlers` is one stable object (the panel keeps it in a ref-backed
//        memo), so it never defeats the memo; per-row booleans (`active`,
//        `focused`, `dropHint`) change for at most a couple of rows at a time.
//        Rows position themselves absolutely at `top`: the virtualizer gives each
//        a fixed height, no measuring.
// WHERE: src/features/schema/sidebar/TableTreeList.tsx, src/lib/tableTree.ts
// ============================================================================

type Of<K extends TreeRow["kind"]> = Extract<TreeRow, { kind: K }>;

export interface RowHandlers {
  focusRow: (key: string) => void;
  openTable: (row: Of<"table">, newTab: boolean) => void;
  toggleColumns: (row: Of<"table">) => void;
  toggleGroup: (row: Of<"group">) => void;
  rowMenu: (event: ReactMouseEvent, row: TreeRow) => void;
  /// The "⋯" on a group header: menu opens under the button.
  buttonMenu: (element: Element, row: TreeRow) => void;
  tablePointerDown: (event: ReactPointerEvent, row: Of<"table">) => void;
  groupPointerDown: (event: ReactPointerEvent, row: Of<"group">) => void;
  refreshColumns: (table: TableInfo) => void;
}

export interface RowViewProps {
  row: TreeRow;
  top: number;
  height: number;
  domId: string;
  /// The row the keyboard is on, and the list has focus.
  focused: boolean;
  /// The table open in the active tab.
  active: boolean;
  /// Tag groups only: a reorder drag is hovering this header.
  dropHint: DropPlacement | null;
  handlers: RowHandlers;
}

const BASE = "absolute left-0 flex w-full cursor-default items-center";

export const TreeRowView = memo(function TreeRowView({ row, top, height, domId, focused, active, dropHint, handlers }: RowViewProps) {
  const position = { top, height };
  switch (row.kind) {
    case "schema":
      return (
        <div
          id={domId}
          role="treeitem"
          aria-level={1}
          aria-selected={false}
          style={position}
          className={cn(BASE, "gap-1.5 px-2.5 text-[10px] font-semibold uppercase tracking-wider text-muted/80", focused ? "bg-surface-secondary/70 text-foreground" : "")}
          onClick={() => { handlers.focusRow(row.key); }}
          onContextMenu={(event) => { handlers.rowMenu(event, row); }}
        >
          <Icon name="folder" size={11} />
          <span className="truncate">{row.name}</span>
          <Badge size="sm" variant="soft" className="ml-auto font-mono text-[10px]">
            {row.count}
          </Badge>
        </div>
      );

    case "group": {
      const tag = row.tag;
      return (
        <div
          id={domId}
          role="treeitem"
          aria-level={row.parent === null ? 1 : 2}
          aria-expanded={!row.collapsed}
          aria-selected={false}
          style={position}
          data-drop={tag === null ? undefined : "tag"}
          data-drop-id={tag?.id}
          className={cn(
            BASE,
            "group gap-1.5 px-2.5 text-xs font-medium text-muted hover:text-foreground",
            focused ? "bg-surface-secondary/70 text-foreground" : "hover:bg-surface-secondary/40",
            dropHint === "before" ? "shadow-[inset_0_2px_0_0_var(--color-accent)]" : "",
            dropHint === "after" ? "shadow-[inset_0_-2px_0_0_var(--color-accent)]" : "",
          )}
          onClick={() => {
            handlers.focusRow(row.key);
            handlers.toggleGroup(row);
          }}
          onContextMenu={(event) => { if (tag !== null) handlers.rowMenu(event, row); }}
          onPointerDown={(event) => { if (tag !== null) handlers.groupPointerDown(event, row); }}
        >
          <Icon name={row.collapsed ? "chevron-right" : "chevron-down"} size={11} className={row.count === 0 ? "opacity-30" : ""} />
          {tag === null ? (
            <span aria-hidden className="size-2 shrink-0 rounded-full border border-muted/60" />
          ) : (
            <span aria-hidden className={cn("size-2 shrink-0 rounded-full", connectionColorMeta(tag.color).fill)} />
          )}
          <span className="truncate">{tag === null ? "Untagged" : tag.name}</span>
          <span className="shrink-0 font-mono text-[10px] text-muted/70">{row.count}</span>
          {tag !== null ? (
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Actions for tag ${tag.name}`}
              tabIndex={-1}
              onPointerDown={(event) => { event.stopPropagation(); }}
              onClick={(event) => {
                event.stopPropagation();
                handlers.buttonMenu(event.currentTarget, row);
              }}
              className="ml-auto size-5 min-w-5 rounded-sm p-0 text-muted opacity-0 group-hover:opacity-100"
            >
              <Icon name="more" size={13} />
            </Button>
          ) : null}
        </div>
      );
    }

    case "table": {
      const table = row.table;
      const inGroup = row.parent?.startsWith("group:") === true;
      return (
        <div
          id={domId}
          role="treeitem"
          aria-level={inGroup ? 3 : 2}
          {...(row.expandable ? { "aria-expanded": row.expanded } : {})}
          aria-selected={active}
          style={{ ...position, paddingLeft: inGroup ? 22 : 10 }}
          title={table.kind === "view" ? "view" : "table"}
          className={cn(
            BASE,
            "group gap-1 rounded-md pr-2 text-xs liquid-hover",
            active ? "glass-pill font-medium text-accent shadow-xs" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
            focused ? "ring-1 ring-ring/70" : "",
          )}
          onClick={(event) => {
            handlers.focusRow(row.key);
            handlers.openTable(row, event.shiftKey);
          }}
          onContextMenu={(event) => { handlers.rowMenu(event, row); }}
          onPointerDown={(event) => {
            // A press on the chevron is a click, not the start of a drag.
            if (event.target instanceof Element && event.target.closest("button") !== null) return;
            handlers.tablePointerDown(event, row);
          }}
        >
          {row.expandable ? (
            <Button
              variant="ghost"
              size="sm"
              tabIndex={-1}
              aria-label={row.expanded ? "Collapse columns" : "Expand columns"}
              onClick={(event) => {
                event.stopPropagation();
                handlers.focusRow(row.key);
                handlers.toggleColumns(row);
              }}
              className="size-4.5 min-w-4.5 shrink-0 rounded-sm p-0 text-muted"
            >
              <Icon name={row.expanded ? "chevron-down" : "chevron-right"} size={11} />
            </Button>
          ) : null}
          <Icon name={table.kind === "view" ? "view" : "table"} size={13} className="shrink-0 opacity-70" />
          <span className="truncate">{table.name}</span>
          <TagDots tags={row.tags} />
          {table.rowEstimate !== null ? (
            <span className="ml-auto shrink-0 rounded-sm px-1 font-mono text-[10px] tabular-nums text-muted/70 opacity-0 transition-opacity group-hover:opacity-100">
              {formatCount(table.rowEstimate)}
            </span>
          ) : null}
        </div>
      );
    }

    case "columns-header":
      return (
        <div id={domId} role="presentation" style={{ ...position, paddingLeft: 40 }} className={cn(BASE, "justify-between pr-2 text-[10px] uppercase tracking-wide text-muted/60")}>
          <span>{row.count} {row.count === 1 ? "column" : "columns"}</span>
          <Button
            variant="ghost"
            size="sm"
            tabIndex={-1}
            aria-label={`Refresh columns of ${row.table.name}`}
            title="Refresh columns"
            onClick={() => { handlers.refreshColumns(row.table); }}
            className="size-4.5 min-w-4.5 rounded-sm p-0 text-muted"
          >
            <Icon name="refresh" size={10} />
          </Button>
        </div>
      );

    case "columns-loading":
      return (
        <div id={domId} role="presentation" style={{ ...position, paddingLeft: 40 }} className={cn(BASE, "gap-2 text-[11px] text-muted")}>
          <Spinner size="sm" /> loading…
        </div>
      );

    case "column":
      return (
        <div
          id={domId}
          role="treeitem"
          aria-level={inGroupLevel(row)}
          aria-selected={false}
          style={{ ...position, paddingLeft: 34 }}
          title={row.column.dataType}
          className={cn(
            BASE,
            "gap-1.5 rounded-md pr-2 text-[11px] text-muted transition-colors hover:bg-surface-secondary/40 hover:text-foreground",
            focused ? "ring-1 ring-ring/70" : "",
          )}
          onClick={() => { handlers.focusRow(row.key); }}
          onContextMenu={(event) => { handlers.rowMenu(event, row); }}
        >
          <Icon name={typeIcon(row.column.dataType, row.column.primaryKey)} size={11} className={row.column.primaryKey ? "text-warning" : "opacity-60"} />
          <span className="truncate font-sans">{row.column.name}</span>
          <span className="ml-auto truncate font-mono text-[10px] text-muted/60">{row.column.dataType}</span>
        </div>
      );
  }
});

function inGroupLevel(row: Of<"column">): number {
  return row.parent?.includes("group:") === true ? 4 : 3;
}

// WHAT:  Up to three small dots in the tags' colours after a table's name.
// WHY:   In A-Z / Z-A the grouping is gone, so the dots are how a tagged table
//        stays recognisable; the names are in the tooltip, never colour alone.
function TagDots({ tags }: { tags: readonly TableTag[] }) {
  if (tags.length === 0) return null;
  const shown = tags.slice(0, 3);
  return (
    <span className="ml-1 flex shrink-0 items-center gap-0.5" title={tags.map((t) => t.name).join(", ")}>
      {shown.map((tag) => (
        <span key={tag.id} aria-hidden className={cn("size-1.5 rounded-full", connectionColorMeta(tag.color).fill)} />
      ))}
      {tags.length > shown.length ? <span className="text-[10px] text-muted/70">+{tags.length - shown.length}</span> : null}
    </span>
  );
}
