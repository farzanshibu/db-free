// SOT: queries-panel, saved-queries-sidebar, recent-queries
import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import type { HistoryEntry, LibraryFolder, SavedQuery } from "@/lib/bindings";
import { savedQueryEntity } from "@/lib/home";
import { ipc, normalizeError } from "@/lib/ipc";
import { buildTree, folderIdOf, itemsInFolder, itemsUnder, nameTaken, siblingFolderNames, type SortDirection } from "@/lib/libraryTree";
import { pickSqlSavePath } from "@/lib/native";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { useLibrary } from "@/stores/library";
import { IconButton } from "@/components/global/Button";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { ConnectionSwitcher } from "@/features/shell/ConnectionSwitcher";
import { ConfirmDeleteDialog, ConnectionPickerDialog, MoveToFolderDialog, NameDialog } from "@/features/documents/LibraryDialogs";
import { LibraryTreeView, type ItemRowContext } from "@/features/documents/LibraryTree";
import { openHomeEntity } from "@/features/home/openEntity";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { QueryVersionsDialog } from "./QueryVersionsDialog";

type DialogState =
  | { kind: "new-folder"; parentId: string | null }
  | { kind: "rename-folder"; folder: LibraryFolder }
  | { kind: "delete-folder"; folder: LibraryFolder }
  | { kind: "move-folder"; folder: LibraryFolder }
  | { kind: "rename"; query: SavedQuery }
  | { kind: "delete"; query: SavedQuery }
  | { kind: "move"; query: SavedQuery }
  | { kind: "copy"; query: SavedQuery }
  | { kind: "versions"; query: SavedQuery };

function slug(text: string): string {
  const cleaned = text.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return cleaned.length > 0 ? cleaned : "queries";
}

// WHAT:  Sidebar of saved queries as a folder tree (A-Z / Z-A, search, "this
//        connection"), plus the most recent distinct statements from history.
//        Click opens a query tab seeded with the SQL; the context menu renames,
//        duplicates, copies to another connection, moves, shows the version
//        history, exports as .sql, pins to Home or deletes.
// WHY:   Saved queries pile up; folders, versions and duplicate/rename/copy are
//        what keep a library of them usable.
// HOW:   The tree is built by `buildTree` from the library store's folders and
//        placements joined with the workspace's `savedQueries`. Every
//        destructive action asks first.
// WHERE: src/stores/library.ts, src/lib/libraryTree.ts, src-tauri/src/commands/library.rs
export function QueriesPanel() {
  const connection = useActiveConnection();
  const savedQueries = useWorkspace((s) => s.savedQueries);
  const loadSavedQueries = useWorkspace((s) => s.loadSavedQueries);
  const deleteSavedQuery = useWorkspace((s) => s.deleteSavedQuery);
  const openQuery = useWorkspace((s) => s.openQuery);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const pins = useWorkspace((s) => s.pins);
  const pinItem = useWorkspace((s) => s.pinItem);
  const unpinItem = useWorkspace((s) => s.unpinItem);
  const folders = useLibrary((s) => s.folders);
  const placements = useLibrary((s) => s.placements);
  const open = useLibrary((s) => s.open.saved_query);
  const thisConnectionOnly = useLibrary((s) => s.thisConnectionOnly.saved_query);
  const library = useLibrary;
  const menu = useContextMenu();
  const [order, setOrder] = useState<SortDirection>("az");
  const [search, setSearch] = useState("");
  const [recent, setRecent] = useState<HistoryEntry[]>([]);
  const [dialog, setDialog] = useState<DialogState | null>(null);

  const cid = connection?.id;

  useEffect(() => {
    void (async () => {
      try {
        await library.getState().loadLibrary();
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  }, [library, showError]);

  useEffect(() => {
    if (!cid) return;
    const token = { cancelled: false };
    void (async () => {
      try {
        const rows = await ipc("list_history", { connectionId: cid, origin: "user", limit: 200 });
        if (token.cancelled) return;
        const seen = new Set<string>();
        const deduped: HistoryEntry[] = [];
        for (const r of rows) {
          const key = r.sql.trim();
          if (seen.has(key)) continue;
          seen.add(key);
          deduped.push(r);
          if (deduped.length >= 20) break;
        }
        setRecent(deduped);
      } catch {
        // history unavailable; not fatal
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [cid]);

  const needle = search.trim().toLowerCase();
  const filtering = thisConnectionOnly && cid !== undefined;
  const tree = useMemo(
    () =>
      buildTree("saved_query", folders, placements, savedQueries, {
        visible: (q) => {
          if (filtering && q.connectionId !== null && q.connectionId !== cid) return false;
          return needle.length === 0 || q.name.toLowerCase().includes(needle) || q.sql.toLowerCase().includes(needle);
        },
        hideEmptyFolders: needle.length > 0,
        direction: order,
      }),
    [folders, placements, savedQueries, filtering, cid, needle, order],
  );
  const openSet = useMemo(() => new Set(open), [open]);
  // Statements already saved as a query are not offered again under "Recent".
  const savedSql = useMemo(() => new Set(savedQueries.map((q) => q.sql.trim())), [savedQueries]);
  const visibleRecent = recent.filter((r) => !savedSql.has(r.sql.trim()));

  if (!cid) return null;

  const isPinned = (q: SavedQuery) => pins.some((p) => p.entity.kind === "saved_query" && p.entity.itemId === q.id);
  const guard = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };
  const namesIn = (connectionId: string | null, folderId: string | null, exceptId?: string) =>
    savedQueries
      .filter((q) => q.connectionId === connectionId && q.id !== exceptId && folderIdOf("saved_query", q.id, placements, folders) === folderId)
      .map((q) => q.name);
  const reload = () => Promise.all([loadSavedQueries(), library.getState().loadLibrary()]);

  const openSaved = (q: SavedQuery) => {
    const entity = savedQueryEntity(q, cid);
    if (entity !== null) void openHomeEntity(entity);
    else openQuery(cid, q.sql, q.name);
  };

  // WHAT:  Writes the chosen queries to one `.sql` file the user names.
  const exportQueries = (ids: string[], baseName: string) =>
    guard(async () => {
      if (ids.length === 0) {
        showInfo("There are no queries to export.");
        return;
      }
      const path = await pickSqlSavePath(baseName);
      if (path === null) return;
      const sql = await ipc("export_saved_queries", { ids });
      await ipc("save_sql_file", { path, sql });
      showInfo(ids.length === 1 ? "Exported 1 query." : `Exported ${ids.length} queries.`);
    });

  const openQueryMenu = (event: ReactMouseEvent, q: SavedQuery) => {
    const entity = savedQueryEntity(q, cid);
    const pinned = isPinned(q);
    const entries: MenuEntry[] = [
      { id: "open", label: "Open in a query tab", icon: "terminal" },
      { id: "copy-sql", label: "Copy SQL", icon: "copy", group: "copy" },
      { id: "copy-name", label: "Copy name", icon: "copy", group: "copy" },
      { id: "pin", label: pinned ? "Unpin from Home" : "Pin to Home", icon: "pin", group: "organize", disabled: entity === null },
      { id: "rename", label: "Rename…", icon: "pencil", group: "organize" },
      { id: "move", label: "Move to folder…", icon: "folder", group: "organize" },
      { id: "duplicate", label: "Duplicate", icon: "copy", group: "duplicate" },
      { id: "copy-to", label: "Copy to connection…", icon: "plug", group: "duplicate" },
      { id: "versions", label: "Version history…", icon: "history", group: "versions" },
      { id: "export", label: "Export as .sql…", icon: "download", group: "versions" },
      { id: "delete", label: "Delete saved query…", icon: "trash", danger: true, group: "danger" },
    ];
    menu.open(event, entries, (action) => {
      if (action === "open") openSaved(q);
      else if (action === "copy-sql") {
        void navigator.clipboard.writeText(q.sql);
        showInfo("Query copied.");
      } else if (action === "copy-name") void navigator.clipboard.writeText(q.name);
      else if (action === "pin") {
        if (entity !== null) void (pinned ? unpinItem("saved_query", q.id) : pinItem(entity));
      } else if (action === "rename") setDialog({ kind: "rename", query: q });
      else if (action === "move") setDialog({ kind: "move", query: q });
      else if (action === "duplicate") {
        void guard(async () => {
          const copy = await ipc("duplicate_saved_query", { id: q.id, targetConnectionId: null, name: null });
          await reload();
          showInfo(`Created “${copy.name}”.`);
        });
      } else if (action === "copy-to") setDialog({ kind: "copy", query: q });
      else if (action === "versions") setDialog({ kind: "versions", query: q });
      else if (action === "export") void exportQueries([q.id], slug(q.name));
      else if (action === "delete") setDialog({ kind: "delete", query: q });
    });
  };

  const openFolderMenu = (event: ReactMouseEvent, folder: LibraryFolder) => {
    menu.open(
      event,
      [
        { id: "new-folder", label: "New folder inside…", icon: "folder" },
        { id: "rename", label: "Rename folder…", icon: "pencil", group: "folder" },
        { id: "move", label: "Move to folder…", icon: "folder", group: "folder" },
        { id: "export", label: "Export folder as .sql…", icon: "download", group: "folder" },
        { id: "delete", label: "Delete folder…", icon: "trash", danger: true, group: "danger" },
      ],
      (action) => {
        if (action === "new-folder") setDialog({ kind: "new-folder", parentId: folder.id });
        else if (action === "rename") setDialog({ kind: "rename-folder", folder });
        else if (action === "move") setDialog({ kind: "move-folder", folder });
        else if (action === "export") {
          const ids = itemsInFolder("saved_query", folder.id, folders, placements, savedQueries).map((q) => q.id);
          void exportQueries(ids, slug(folder.name));
        } else if (action === "delete") setDialog({ kind: "delete-folder", folder });
      },
    );
  };

  const renderQuery = (q: SavedQuery, context: ItemRowContext) => (
    <div
      key={q.id}
      title={q.sql}
      {...context.dragProps}
      onContextMenu={(event) => openQueryMenu(event, q)}
      className="group flex h-6 items-center gap-1.5 rounded-md pr-1 text-xs text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
      style={{ paddingLeft: 8 + context.depth * 14 }}
    >
      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          if (!context.justDragged()) openSaved(q);
        }}
        className="flex h-auto min-w-0 flex-1 items-center justify-start gap-2 bg-transparent p-0 text-left hover:bg-transparent"
      >
        <Icon name="file" size={13} className="shrink-0 text-accent" />
        <span className="truncate">{q.name}</span>
        {q.tags.length > 0 ? (
          <Badge size="sm" variant="soft" className="ml-auto max-w-24 truncate text-[10px]">
            {q.tags.join(", ")}
          </Badge>
        ) : null}
      </Button>
      {isPinned(q) ? <Icon name="pin" size={11} className="shrink-0 text-accent/80" aria-label="Pinned to Home" /> : null}
      <span className="flex shrink-0 items-center opacity-0 transition-opacity group-hover:opacity-100">
        <Button variant="ghost" size="icon-sm" aria-label={`Version history of ${q.name}`} onClick={() => setDialog({ kind: "versions", query: q })}>
          <Icon name="history" size={13} />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label={`${q.name} actions`} onClick={(event) => openQueryMenu(event, q)}>
          <Icon name="more" size={14} />
        </Button>
      </span>
    </div>
  );

  const folderCount = folders.filter((f) => f.scope === "saved_query").length;
  const exportVisibleIds = [...tree.items, ...tree.folders.flatMap((node) => itemsUnder(node))].map((q) => q.id);

  return (
    <aside className="flex h-full w-full flex-col glass-sidebar select-none">
      <div className="@container flex app-toolbar shrink-0 items-center gap-1.5 overflow-hidden border-b border-border/40 glass-header">
        <ConnectionSwitcher caption="Saved queries" />
        <div className="min-w-0 flex-1" />
        <span className="flex shrink-0 items-center gap-0.5">
          <IconButton icon="refresh" label="Refresh" onClick={() => void guard(async () => { await reload(); })} />
          <IconButton icon="folder" label="New folder" onClick={() => setDialog({ kind: "new-folder", parentId: null })} />
          <IconButton icon="download" label="Export queries as .sql" onClick={() => void exportQueries(exportVisibleIds, slug(connection.name))} />
          <IconButton icon="plus" label="New query" onClick={() => openQuery(cid)} />
        </span>
      </div>
      <div className="flex flex-col gap-2 px-2.5 py-1.5">
        <SearchInput value={search} onChange={setSearch} aria-label="Search saved queries" placeholder="Search name or SQL…" className="glass-input w-full rounded-md text-xs" />
        <div className="flex items-center gap-2">
          <Segmented
            label="Show queries from"
            value={filtering ? "this" : "all"}
            options={[
              { value: "this", label: "This connection" },
              { value: "all", label: "All" },
            ]}
            onChange={(value) => library.getState().setThisConnectionOnly("saved_query", value === "this")}
          />
          <Segmented label="Sort" value={order} onChange={setOrder} options={[{ value: "az", label: "A-Z" }, { value: "za", label: "Z-A" }]} />
          <Badge size="sm" variant="soft" className="ml-auto font-mono text-[9.5px]">
            {tree.count}
          </Badge>
        </div>
      </div>
      <ScrollArea className="min-h-0 flex-1 px-1.5 py-1">
        {savedQueries.length === 0 && folderCount === 0 ? (
          <p className="px-2.5 py-1.5 text-xs text-muted">No saved queries yet. Use Save in a query tab.</p>
        ) : tree.count === 0 && tree.folders.length === 0 ? (
          <p className="px-2.5 py-1.5 text-xs text-muted">{needle.length > 0 ? `No saved query matches “${search.trim()}”.` : "Nothing saved for this connection. Switch to “All” to see every saved query."}</p>
        ) : null}
        <LibraryTreeView
          tree={tree}
          open={openSet}
          forceOpen={needle.length > 0}
          onToggle={(folderId, next) => library.getState().setFolderOpen("saved_query", folderId, next)}
          onFolderMenu={openFolderMenu}
          renderItem={renderQuery}
          onMoveItem={(itemId, folderId) => {
            if (folderIdOf("saved_query", itemId, placements, folders) === folderId) return;
            void guard(() => library.getState().moveItem("saved_query", itemId, folderId));
          }}
          onMoveFolder={(folderId, parentId) => void guard(() => library.getState().moveFolder(folderId, parentId))}
        />
        <div className="mt-2 px-2.5 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted/80">Recent</div>
        {visibleRecent.length === 0 ? <p className="px-2.5 py-1 text-xs text-muted">Nothing run yet.</p> : null}
        {visibleRecent.map((r) => (
          <div
            key={r.id}
            title={r.sql}
            className="w-full"
            onContextMenu={(e) =>
              menu.open(
                e,
                [
                  { id: "open", label: "Open in a query tab", icon: "terminal" },
                  { id: "copy", label: "Copy SQL", icon: "copy" },
                ],
                (action) => {
                  if (action === "open") openQuery(cid, r.sql);
                  else {
                    void navigator.clipboard.writeText(r.sql);
                    showInfo("Query copied.");
                  }
                },
              )
            }
          >
            <Button
              variant="ghost"
              size="sm"
              onClick={() => openQuery(cid, r.sql)}
              className="flex h-6 w-full items-center justify-start gap-2 rounded-md px-2 text-left text-[11px] text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            >
              <Icon name="file" size={12} className="shrink-0 opacity-60" />
              <span className="truncate font-mono">{r.sql.replace(/\s+/g, " ")}</span>
            </Button>
          </div>
        ))}
      </ScrollArea>
      {menu.node}

      {dialog?.kind === "new-folder" ? (
        <NameDialog
          title={dialog.parentId === null ? "New folder" : "New folder inside"}
          label="Folder name"
          placeholder="Reports"
          confirmLabel="Create"
          problem={(name) => (nameTaken(siblingFolderNames("saved_query", dialog.parentId, folders), name) ? `A folder named “${name}” already exists here.` : null)}
          onSubmit={async (name) => {
            await library.getState().createFolder("saved_query", dialog.parentId, name);
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
          problem={(name) => (nameTaken(siblingFolderNames("saved_query", dialog.folder.parentId, folders, dialog.folder.id), name) ? `A folder named “${name}” already exists here.` : null)}
          onSubmit={(name) => library.getState().renameFolder(dialog.folder.id, name)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "move-folder" ? (
        <MoveToFolderDialog
          scope="saved_query"
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
          body="The folder is removed; everything inside it moves up one level. No saved query is deleted."
          confirmLabel="Delete folder"
          onConfirm={() => library.getState().deleteFolder(dialog.folder.id)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "rename" ? (
        <NameDialog
          title="Rename saved query"
          label="Name"
          initial={dialog.query.name}
          confirmLabel="Rename"
          problem={(name) =>
            nameTaken(namesIn(dialog.query.connectionId, folderIdOf("saved_query", dialog.query.id, placements, folders), dialog.query.id), name)
              ? `A saved query named “${name}” already exists in this folder.`
              : null
          }
          onSubmit={async (name) => {
            await ipc("rename_saved_query", { id: dialog.query.id, name });
            await loadSavedQueries();
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "move" ? (
        <MoveToFolderDialog
          scope="saved_query"
          title={`Move “${dialog.query.name}”`}
          current={folderIdOf("saved_query", dialog.query.id, placements, folders)}
          onMove={(folderId) => library.getState().moveItem("saved_query", dialog.query.id, folderId)}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "copy" ? (
        <ConnectionPickerDialog
          title={`Copy “${dialog.query.name}” to a connection`}
          description="A copy of this query is saved on the connection you pick, in the same folder. The original stays where it is."
          exclude={dialog.query.connectionId}
          onPick={async (targetConnectionId) => {
            const copy = await ipc("duplicate_saved_query", { id: dialog.query.id, targetConnectionId, name: null });
            await reload();
            showInfo(`Copied “${copy.name}”.`);
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "delete" ? (
        <ConfirmDeleteDialog
          title={`Delete “${dialog.query.name}”?`}
          body="This saved query and its version history are deleted for good."
          confirmLabel="Delete query"
          onConfirm={async () => {
            await deleteSavedQuery(dialog.query.id);
            await library.getState().loadLibrary();
            await useWorkspace.getState().loadPins();
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === "versions" ? (
        <QueryVersionsDialog
          query={dialog.query}
          onOpenVersion={(sql, label) => {
            openQuery(dialog.query.connectionId ?? cid, sql, label);
            setDialog(null);
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
    </aside>
  );
}
