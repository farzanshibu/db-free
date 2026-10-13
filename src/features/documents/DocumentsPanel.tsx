// SOT: documents-panel, dashboards-sidebar, workflows-sidebar, diagrams-sidebar
import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import type { Document, DocumentBody, DocumentKind, LibraryFolder, LibraryScope } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { documentEntity } from "@/lib/home";
import { folderIdOf, buildTree, nameTaken, siblingFolderNames } from "@/lib/libraryTree";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { useLibrary } from "@/stores/library";
import { IconButton } from "@/components/global/Button";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { Segmented } from "@/components/global/Field";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { ConnectionSwitcher } from "@/features/shell/ConnectionSwitcher";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ConfirmDeleteDialog, ConnectionPickerDialog, MoveToFolderDialog, NameDialog } from "./LibraryDialogs";
import { LibraryTreeView, type ItemRowContext } from "./LibraryTree";

const META: Record<DocumentKind, { title: string; singular: string; icon: IconName; empty: string }> = {
  dashboard: { title: "Dashboards", singular: "dashboard", icon: "columns", empty: "No dashboards yet. Create one to chart query results." },
  workflow: { title: "Workflows", singular: "workflow", icon: "play", empty: "No workflows yet. Chain SQL steps and run them in order." },
  diagram: { title: "Schema Diagrams", singular: "diagram", icon: "view", empty: "No diagrams yet. Design tables and relations on a canvas." },
};

function emptyBody(kind: DocumentKind): DocumentBody {
  switch (kind) {
    case "dashboard":
      return { kind: "dashboard", data: { widgets: [], variables: [], refreshSeconds: 0 } };
    case "workflow":
      return { kind: "workflow", data: { steps: [] } };
    case "diagram":
      return { kind: "diagram", data: { tables: [], relations: [] } };
  }
}

type DialogState =
  | { kind: "new-folder"; parentId: string | null }
  | { kind: "rename-folder"; folder: LibraryFolder }
  | { kind: "delete-folder"; folder: LibraryFolder }
  | { kind: "move-folder"; folder: LibraryFolder }
  | { kind: "rename"; doc: Document }
  | { kind: "delete"; doc: Document }
  | { kind: "move"; doc: Document }
  | { kind: "copy"; doc: Document };

// WHAT:  Sidebar listing one document kind (dashboards, workflows or diagrams) as a
//        folder tree, with the actions a library list offers: new folder, rename,
//        duplicate, copy to another connection, move, pin to Home, delete.
// WHY:   The three kinds share their lifecycle, so they share this list; only the
//        icon, the empty text and the new-document body differ.
// HOW:   The tree comes from the library store's folders and placements joined with
//        this kind's documents. "This connection" shows the active connection's
//        documents and the shared ones; "All" shows everything.
// WHERE: src/stores/library.ts, src/lib/libraryTree.ts, src-tauri/src/commands/library.rs
export function DocumentsPanel({ kind }: { kind: DocumentKind }) {
  const connection = useActiveConnection();
  const docs = useWorkspace((s) => s.documents[kind]);
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const deleteDocument = useWorkspace((s) => s.deleteDocument);
  const openDocument = useWorkspace((s) => s.openDocument);
  const activeTabId = useWorkspace((s) => s.activeTabId);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const pins = useWorkspace((s) => s.pins);
  const pinItem = useWorkspace((s) => s.pinItem);
  const unpinItem = useWorkspace((s) => s.unpinItem);
  const folders = useLibrary((s) => s.folders);
  const placements = useLibrary((s) => s.placements);
  const open = useLibrary((s) => s.open[kind]);
  const thisConnectionOnly = useLibrary((s) => s.thisConnectionOnly[kind]);
  const library = useLibrary;
  const scope: LibraryScope = kind;
  const meta = META[kind];
  const menu = useContextMenu();
  const [dialog, setDialog] = useState<DialogState | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        await Promise.all([loadDocuments(kind), library.getState().loadLibrary()]);
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  }, [kind, loadDocuments, library, showError]);

  const connectionId = connection?.id ?? null;
  const filtering = thisConnectionOnly && connectionId !== null;
  const tree = useMemo(
    () =>
      buildTree(scope, folders, placements, docs, {
        visible: (d) => !filtering || d.connectionId === null || d.connectionId === connectionId,
        hideEmptyFolders: false,
        direction: "az",
      }),
    [scope, folders, placements, docs, filtering, connectionId],
  );
  const openSet = useMemo(() => new Set(open), [open]);
  const pinned = (d: Document) => pins.some((p) => p.entity.kind === kind && p.entity.itemId === d.id);

  const guard = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const create = async (folderId: string | null) => {
    const name = `${meta.title.replace(/s$/, "")} ${docs.length + 1}`;
    const doc: Document = { id: "", kind, connectionId, name, body: emptyBody(kind), tags: [], createdAt: "", updatedAt: "" };
    await guard(async () => {
      const saved = await saveDocument(doc);
      if (folderId !== null) await library.getState().moveItem(scope, saved.id, folderId);
      openDocument(kind, saved.id, saved.connectionId);
    });
  };

  const reload = async () => {
    await Promise.all([loadDocuments(kind), library.getState().loadLibrary()]);
  };

  const openDocMenu = (event: ReactMouseEvent, doc: Document) => {
    const entity = documentEntity(doc, connectionId);
    const isPinned = pinned(doc);
    const entries: MenuEntry[] = [
      { id: "open", label: `Open ${meta.singular}`, icon: meta.icon },
      { id: "pin", label: isPinned ? "Unpin from Home" : "Pin to Home", icon: "pin", group: "organize", disabled: entity === null },
      { id: "rename", label: "Rename…", icon: "pencil", group: "organize" },
      { id: "move", label: "Move to folder…", icon: "folder", group: "organize" },
      { id: "duplicate", label: "Duplicate", icon: "copy", group: "copy" },
      { id: "copy", label: "Copy to connection…", icon: "plug", group: "copy" },
      { id: "delete", label: `Delete ${meta.singular}…`, icon: "trash", danger: true, group: "danger" },
    ];
    menu.open(event, entries, (action) => {
      if (action === "open") openDocument(kind, doc.id, doc.connectionId);
      else if (action === "pin") {
        if (entity !== null) void (isPinned ? unpinItem(kind, doc.id) : pinItem(entity));
      } else if (action === "rename") setDialog({ kind: "rename", doc });
      else if (action === "move") setDialog({ kind: "move", doc });
      else if (action === "duplicate") {
        void guard(async () => {
          const copy = await ipc("duplicate_document", { id: doc.id, targetConnectionId: null });
          await reload();
          showInfo(`Created “${copy.name}”.`);
        });
      } else if (action === "copy") setDialog({ kind: "copy", doc });
      else if (action === "delete") setDialog({ kind: "delete", doc });
    });
  };

  const openFolderMenu = (event: ReactMouseEvent, folder: LibraryFolder) => {
    menu.open(
      event,
      [
        { id: "new-doc", label: `New ${meta.singular} here`, icon: "plus" },
        { id: "new-folder", label: "New folder inside…", icon: "folder", group: "folder" },
        { id: "rename", label: "Rename folder…", icon: "pencil", group: "folder" },
        { id: "move", label: "Move to folder…", icon: "folder", group: "folder" },
        { id: "delete", label: "Delete folder…", icon: "trash", danger: true, group: "danger" },
      ],
      (action) => {
        if (action === "new-doc") void create(folder.id);
        else if (action === "new-folder") setDialog({ kind: "new-folder", parentId: folder.id });
        else if (action === "rename") setDialog({ kind: "rename-folder", folder });
        else if (action === "move") setDialog({ kind: "move-folder", folder });
        else if (action === "delete") setDialog({ kind: "delete-folder", folder });
      },
    );
  };

  const renderDoc = (d: Document, context: ItemRowContext) => {
    const active = activeTabId === `doc:${kind}:${d.id}`;
    return (
      <div
        key={d.id}
        {...context.dragProps}
        onContextMenu={(event) => openDocMenu(event, d)}
        className={cn(
          "group flex h-6 items-center gap-1.5 rounded-md pr-1 text-xs liquid-hover",
          active ? "glass-pill text-accent" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
        )}
        style={{ paddingLeft: 8 + context.depth * 14 }}
      >
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            if (!context.justDragged()) openDocument(kind, d.id, d.connectionId);
          }}
          className="flex h-auto min-w-0 flex-1 items-center justify-start gap-2 bg-transparent p-0 text-left hover:bg-transparent"
        >
          <Icon name={meta.icon} size={13} className={cn("shrink-0", active ? "text-accent" : "text-muted")} />
          <span className="truncate font-medium">{d.name}</span>
        </Button>
        {pinned(d) ? <Icon name="pin" size={11} className="shrink-0 text-accent/80" aria-label="Pinned to Home" /> : null}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`${d.name} actions`}
          className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          onClick={(event) => openDocMenu(event, d)}
        >
          <Icon name="more" size={14} />
        </Button>
      </div>
    );
  };

  const empty = docs.length === 0 && folders.every((f) => f.scope !== scope);

  return (
    <aside className="flex h-full w-full min-w-0 flex-col glass-sidebar select-none">
      <div className="@container drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-1.5 border-b border-border/40" data-tauri-drag-region>
        <ConnectionSwitcher caption={meta.title} />
        <div className="drag-region h-full min-w-4 flex-1" data-tauri-drag-region />
        <span className="flex shrink-0 items-center gap-0.5">
          <IconButton icon="refresh" label="Refresh" onClick={() => void guard(reload)} />
          <IconButton icon="folder" label="New folder" onClick={() => setDialog({ kind: "new-folder", parentId: null })} />
          <IconButton icon="plus" label={`New ${meta.singular}`} onClick={() => void create(null)} />
        </span>
      </div>
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <Segmented
          label={`Show ${meta.title.toLowerCase()} from`}
          value={filtering ? "this" : "all"}
          options={[
            { value: "this", label: "This connection", disabled: connectionId === null },
            { value: "all", label: "All" },
          ]}
          onChange={(value) => library.getState().setThisConnectionOnly(kind, value === "this")}
        />
        <Badge size="sm" variant="soft" className="ml-auto font-mono text-[9.5px]">
          {tree.count}
        </Badge>
      </div>
      <ScrollArea className="min-h-0 flex-1 px-1.5 py-1">
        {empty ? (
          <p className="px-2.5 py-1.5 text-xs text-muted">{meta.empty}</p>
        ) : tree.count === 0 && tree.folders.length === 0 ? (
          <p className="px-2.5 py-1.5 text-xs text-muted">Nothing for this connection yet. Switch to “All” to see every {meta.singular}.</p>
        ) : null}
        <LibraryTreeView
          tree={tree}
          open={openSet}
          forceOpen={false}
          onToggle={(folderId, next) => library.getState().setFolderOpen(scope, folderId, next)}
          onFolderMenu={openFolderMenu}
          renderItem={renderDoc}
          onMoveItem={(itemId, folderId) => {
            if (folderIdOf(scope, itemId, placements, folders) === folderId) return;
            void guard(() => library.getState().moveItem(scope, itemId, folderId));
          }}
          onMoveFolder={(folderId, parentId) => void guard(() => library.getState().moveFolder(folderId, parentId))}
        />
      </ScrollArea>
      {menu.node}

      {dialog?.kind === "new-folder" ? (
        <NameDialog
          title={dialog.parentId === null ? "New folder" : "New folder inside"}
          label="Folder name"
          placeholder="Reports"
          confirmLabel="Create"
          problem={(name) => (nameTaken(siblingFolderNames(scope, dialog.parentId, folders), name) ? `A folder named “${name}” already exists here.` : null)}
          onSubmit={async (name) => {
            await library.getState().createFolder(scope, dialog.parentId, name);
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "rename-folder" ? (
        <NameDialog
          title="Rename folder"
          label="Folder name"
          initial={dialog.folder.name}
          confirmLabel="Rename"
          problem={(name) => (nameTaken(siblingFolderNames(scope, dialog.folder.parentId, folders, dialog.folder.id), name) ? `A folder named “${name}” already exists here.` : null)}
          onSubmit={(name) => library.getState().renameFolder(dialog.folder.id, name)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "move-folder" ? (
        <MoveToFolderDialog
          scope={scope}
          title={`Move “${dialog.folder.name}”`}
          current={dialog.folder.parentId}
          exclude={dialog.folder.id}
          onMove={(parentId) => library.getState().moveFolder(dialog.folder.id, parentId)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "delete-folder" ? (
        <ConfirmDeleteDialog
          title={`Delete folder “${dialog.folder.name}”?`}
          body={`The folder is removed; everything inside it moves up one level. No ${meta.singular} is deleted.`}
          confirmLabel="Delete folder"
          onConfirm={() => library.getState().deleteFolder(dialog.folder.id)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "rename" ? (
        <NameDialog
          title={`Rename ${meta.singular}`}
          label="Name"
          initial={dialog.doc.name}
          confirmLabel="Rename"
          onSubmit={async (name) => {
            await ipc("rename_document", { id: dialog.doc.id, name });
            await loadDocuments(kind);
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "move" ? (
        <MoveToFolderDialog
          scope={scope}
          title={`Move “${dialog.doc.name}”`}
          current={folderIdOf(scope, dialog.doc.id, placements, folders)}
          onMove={(folderId) => library.getState().moveItem(scope, dialog.doc.id, folderId)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "copy" ? (
        <ConnectionPickerDialog
          title={`Copy “${dialog.doc.name}” to a connection`}
          description={`A copy of this ${meta.singular} is made on the connection you pick. The original stays where it is.`}
          exclude={dialog.doc.connectionId}
          onPick={async (targetConnectionId) => {
            const copy = await ipc("duplicate_document", { id: dialog.doc.id, targetConnectionId });
            await reload();
            showInfo(`Copied “${copy.name}”.`);
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "delete" ? (
        <ConfirmDeleteDialog
          title={`Delete “${dialog.doc.name}”?`}
          body={`This ${meta.singular} is deleted for good. Its open tab closes.`}
          confirmLabel={`Delete ${meta.singular}`}
          onConfirm={async () => {
            await deleteDocument(kind, dialog.doc.id);
            await library.getState().loadLibrary();
            await useWorkspace.getState().loadPins();
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
    </aside>
  );
}
