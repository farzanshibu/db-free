// SOT: library-tree-view, folder-rows, drag-to-folder, drag-ghost
import { Fragment, useMemo, type HTMLAttributes, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { LibraryFolder } from "@/lib/bindings";
import { descendantIds, findNode, type FolderNode, type LibraryTreeData } from "@/lib/libraryTree";
import { usePointerDrag } from "@/lib/pointerDrag";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

// WHAT:  The folder tree the four library sidebars draw: nested folders that open
//        and close, with each list's own item rows inside them.
// WHY:   Folders behave the same in every list — open state, drop targets, the
//        count badge — so the list-specific part is only how one item row looks.
// HOW:   Folders are drop targets (`data-drop-id`); items and folders are dragged
//        with pointer events (src/lib/pointerDrag.ts). The whole folder block —
//        header and contents — is the target, so dropping on an item inside a
//        folder files it into that folder. The blank space is the top level.
// WHERE: src/features/documents/DocumentsPanel.tsx, src/features/queries/QueriesPanel.tsx

const ROOT_DROP = "__top_level__";
const INDENT = 14;

type DragPayload = { kind: "item"; id: string } | { kind: "folder"; id: string };

export interface ItemRowContext {
  /// Nesting depth of the row: 0 at the top level, one more per folder.
  depth: number;
  /// Spread on the row so it can be dragged into a folder.
  dragProps: Pick<HTMLAttributes<HTMLElement>, "onPointerDown">;
  /// True right after a drag: the row's click handler should ignore it.
  justDragged: () => boolean;
}

interface LibraryTreeViewProps<T extends { id: string; name: string }> {
  tree: LibraryTreeData<T>;
  /// Folder ids that are open.
  open: ReadonlySet<string>;
  /// Show everything open (a search is on).
  forceOpen: boolean;
  onToggle: (folderId: string, open: boolean) => void;
  onFolderMenu: (event: ReactMouseEvent, folder: LibraryFolder) => void;
  renderItem: (item: T, context: ItemRowContext) => ReactNode;
  onMoveItem: (itemId: string, folderId: string | null) => void;
  onMoveFolder: (folderId: string, parentId: string | null) => void;
}

export function LibraryTreeView<T extends { id: string; name: string }>({
  tree,
  open,
  forceOpen,
  onToggle,
  onFolderMenu,
  renderItem,
  onMoveItem,
  onMoveFolder,
}: LibraryTreeViewProps<T>) {
  const { drag, begin, wasDragged } = usePointerDrag<DragPayload>({
    onDrop: (payload, overId) => {
      if (overId === null) return;
      const target = overId === ROOT_DROP ? null : overId;
      if (payload.kind === "item") {
        onMoveItem(payload.id, target);
        return;
      }
      const node = findNode(tree.folders, payload.id);
      // A folder cannot go into itself or something beneath it.
      if (node !== undefined && target !== null && descendantIds(node).has(target)) return;
      onMoveFolder(payload.id, target);
    },
  });

  // Which folder blocks the dragged thing may not land on (itself and its subtree).
  const dragged = drag?.payload ?? null;
  const blocked = useMemo(() => {
    if (dragged?.kind !== "folder") return new Set<string>();
    const node = findNode(tree.folders, dragged.id);
    return node === undefined ? new Set<string>() : descendantIds(node);
  }, [dragged, tree.folders]);

  const dropOver = (id: string) => drag !== null && drag.overId === id && !blocked.has(id);

  const renderFolder = (node: FolderNode<T>, depth: number): ReactNode => {
    const { folder } = node;
    const isOpen = forceOpen || open.has(folder.id);
    return (
      <div key={folder.id} data-drop-id={folder.id}>
        <div
          className={cn(
            "group flex h-6 items-center gap-1 rounded-md pr-1 text-xs text-muted liquid-hover hover:bg-surface-secondary/70 hover:text-foreground",
            dropOver(folder.id) ? "bg-accent/15 text-foreground ring-1 ring-accent/50" : "",
          )}
          style={{ paddingLeft: 4 + depth * INDENT }}
          onContextMenu={(event) => onFolderMenu(event, folder)}
          onPointerDown={(event: ReactPointerEvent<HTMLElement>) => begin(event, { kind: "folder", id: folder.id }, folder.name)}
        >
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={isOpen}
            onClick={() => {
              if (!wasDragged() && !forceOpen) onToggle(folder.id, !isOpen);
            }}
            className="flex h-full min-w-0 flex-1 items-center justify-start gap-1.5 bg-transparent p-1 text-left font-medium hover:bg-transparent"
          >
            <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={11} className="shrink-0 opacity-70" />
            <Icon name="folder" size={13} className="shrink-0 text-accent" />
            <span className="truncate">{folder.name}</span>
          </Button>
          <Badge size="sm" variant="soft" className="shrink-0 font-mono text-[9.5px]">
            {node.count}
          </Badge>
        </div>
        {isOpen ? (
          <div>
            {node.folders.map((child) => renderFolder(child, depth + 1))}
            {node.items.map((item) => (
              <Fragment key={item.id}>{renderRow(item, depth + 1)}</Fragment>
            ))}
          </div>
        ) : null}
      </div>
    );
  };

  const renderRow = (item: T, depth: number): ReactNode =>
    renderItem(item, {
      depth,
      dragProps: { onPointerDown: (event: ReactPointerEvent<HTMLElement>) => begin(event, { kind: "item", id: item.id }, item.name) },
      justDragged: wasDragged,
    });

  return (
    <div
      data-drop-id={ROOT_DROP}
      className={cn("min-h-full rounded-lg pb-1.5 transition-colors", dropOver(ROOT_DROP) ? "bg-accent/10 ring-1 ring-accent/40" : "")}
    >
      {tree.folders.map((node) => renderFolder(node, 0))}
      {tree.items.map((item) => (
        <Fragment key={item.id}>{renderRow(item, 0)}</Fragment>
      ))}
      {drag !== null ? (
        // Only while dragging: a labelled strip to drop on to take something out of its folder.
        <div
          data-drop-id={ROOT_DROP}
          className={cn(
            "mx-1 mt-2 flex h-7 items-center justify-center gap-1.5 rounded-md border border-dashed text-xs",
            dropOver(ROOT_DROP) ? "border-accent/60 bg-accent/10 text-foreground" : "border-border/60 text-muted",
          )}
        >
          <Icon name="home" size={12} />
          Drop here for the top level
        </div>
      ) : null}
      {drag !== null
        ? createPortal(
            <div
              className="pointer-events-none fixed z-[60] flex max-w-64 items-center gap-1.5 rounded-md glass-pill px-2 py-1 text-xs text-foreground shadow-lg"
              style={{ left: drag.x + 12, top: drag.y + 10 }}
            >
              <Icon name={drag.payload.kind === "folder" ? "folder" : "file"} size={12} className="shrink-0 text-accent" />
              <span className="truncate">{drag.label}</span>
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
