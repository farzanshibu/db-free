// SOT: tables-panel, sidebar-tables, database-switcher, schema-switcher, table-tree, sidebar-table-actions, sidebar-tag-wiring
import { useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { ColumnInfo, ConnectionSummary, DdlProfile, TableInfo, TableRef, TableTag, TagState } from "@/lib/bindings";
import { collectionNoun, engineMeta, isKeyValueEngine, supportsErd } from "@/lib/engines";
import { Icon } from "@/lib/icons";
import { ipc, normalizeError } from "@/lib/ipc";
import { tableEntity, tableItemId } from "@/lib/home";
import { isRedisEngine } from "@/lib/redisCommands";
import { createTableSketch, qualifiedIdentifier } from "@/lib/sqlQuote";
import { buildRows, isFocusable, reorderIds, schemaScope, SORT_MODES, tagMemberCount, tagsForScope, tagsOfTable, type KeyIntent, type SortMode, type TreeRow } from "@/lib/tableTree";
import { tableKey, useActiveConnection, useWorkspace } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { AppSelect, Segmented } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { dashboardForTablePrompt } from "@/features/chat/prompts";
import type { SchemaAction } from "@/features/schema/actions";
import { SchemaActionHost } from "@/features/schema/SchemaActionHost";
import { beginPointerDrag } from "@/features/schema/pointerDrag";
import { MenuNodes, useAnchoredMenu, type MenuNode } from "@/features/schema/sidebar/AnchoredMenu";
import { TableTreeList, type HoverDrop } from "@/features/schema/sidebar/TableTreeList";
import type { RowHandlers } from "@/features/schema/sidebar/TreeRowView";
import { columnMenu, ddlAllowed, groupMenu, namespaceAction, namespaceMenu, schemaAction, schemaMenu, tableAction, tableMenu } from "@/features/schema/sidebar/tableMenus";
import { DeleteTagDialog, TagDialog, type TagScopeChoice } from "@/features/schema/tags/TagDialogs";
import { tagScopeKey, useTableTags, useTagState } from "@/features/schema/tags/useTableTags";
import { useDdlProfile } from "@/features/schema/useDdlProfile";
import { ConnectionSwitcher } from "./ConnectionSwitcher";
import { KeyTree } from "./KeyTree";
import { beginPaneDrag } from "./paneDrag";

// WHAT:  Sidebar listing the active connection's tables, with database and
//        schema switchers on top (the "saas_db / public" breadcrumb), a sort /
//        tag grouping control, search, and every table-level action.
// WHY:   An empty Database on the connection connects to the default database;
//        every database the server exposes is offered here for switching. The
//        list itself is virtualized (a schema can hold thousands of tables) and
//        keyboard-driven, so this file wires state to the pure row model
//        (`lib/tableTree.ts`) and the row views (`features/schema/sidebar/`).
// HOW:   The panel owns the interaction state (sort mode, search, which tables
//        have their columns open, the focused row, open dialogs) and exposes it to
//        rows through ONE stable handler object that reads the latest state from a
//        ref, so a row's memo is never defeated by a new callback. Forms for
//        create / edit / duplicate / drop are opened as a `SchemaAction` and
//        rendered by `SchemaActionHost`. Tags live in the app's own store.
// WHERE: src/lib/tableTree.ts, src/features/schema/, src-tauri/src/integrations/mod.rs (SessionInfo.databases)
export function TablesPanel() {
  const connection = useActiveConnection();
  if (!connection) return null;
  // Keyed by connection: search, expanded tables and focus belong to one of them.
  return <TablesPanelBody key={connection.id} connection={connection} />;
}

const MODE_KEY = (connectionId: string) => `db-free:tables-sort:${connectionId}`;

function readMode(connectionId: string): SortMode {
  try {
    const raw = localStorage.getItem(MODE_KEY(connectionId));
    const found = SORT_MODES.find((m) => m.value === raw);
    if (found) return found.value;
  } catch {
    // storage unavailable: the default order is fine
  }
  return "az";
}

function writeMode(connectionId: string, mode: SortMode): void {
  try {
    localStorage.setItem(MODE_KEY(connectionId), mode);
  } catch {
    // storage unavailable: the choice lasts until the panel is left
  }
}

type TableRow = Extract<TreeRow, { kind: "table" }>;
type GroupRow = Extract<TreeRow, { kind: "group" }>;
type SchemaRow = Extract<TreeRow, { kind: "schema" }>;
type TagDialogState = { kind: "create"; scope: string; thenTable: string | null } | { kind: "edit"; tag: TableTag };
type PanelHandlers = RowHandlers & { onIntent: (intent: KeyIntent) => void; onMenuKey: (row: TreeRow, element: HTMLElement | null) => void };

/// Everything a handler needs that changes between renders.
interface Live {
  id: string;
  connection: ConnectionSummary;
  database: string;
  speaksSql: boolean;
  keyValue: boolean;
  noun: string;
  columnPreview: boolean;
  profile: DdlProfile | undefined;
  rows: readonly TreeRow[];
  tagState: TagState | undefined;
  columnsCache: Readonly<Record<string, ColumnInfo[]>>;
  tagMode: boolean;
  menu: ReturnType<typeof useAnchoredMenu>;
}

function TablesPanelBody({ connection }: { connection: ConnectionSummary }) {
  const id = connection.id;
  const engine = connection.engine;
  const catalog = useWorkspace((s) => s.catalogs[id]);
  const info = useWorkspace((s) => s.sessionInfos[id]);
  const schemaFilter = useWorkspace((s) => s.schemaFilter[id] ?? null);
  const columnsCache = useWorkspace((s) => s.columnsCache);
  const columnPreview = useWorkspace((s) => s.settings?.columnPreview ?? true);
  const connecting = useWorkspace((s) => s.connecting);
  const activeRef = useWorkspace((s) => {
    const tab = s.tabs.find((t) => t.id === s.activeTabId);
    return tab?.kind === "table" && tab.connectionId === id ? tableKey(tab.table) : null;
  });
  const setSchemaFilter = useWorkspace((s) => s.setSchemaFilter);
  const switchDatabase = useWorkspace((s) => s.switchDatabase);
  const loadCatalog = useWorkspace((s) => s.loadCatalog);
  const openQuery = useWorkspace((s) => s.openQuery);
  const openErd = useWorkspace((s) => s.openErd);

  const menu = useAnchoredMenu();
  const profile = useDdlProfile(id);
  const speaksSql = engineMeta(engine).commandLanguage === "SQL";
  const keyValue = isKeyValueEngine(engine);
  // Redis-compatible engines scan and search their keyspace on the server (KeyTree
  // has its own search box), so the panel's table search has nothing to add there.
  const serverSearch = isRedisEngine(engine);
  const namespaced = info?.capabilities.namespaces ?? true;
  const database = info?.database ?? "";
  const tagState = useTagState(keyValue ? null : id, database, catalog);
  const noun = (profile?.noun ?? collectionNoun(engine).replace(/s$/, "")).toLowerCase();

  const [mode, setModeState] = useState<SortMode>(() => readMode(id));
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [action, setAction] = useState<SchemaAction | null>(null);
  const [tagDialog, setTagDialog] = useState<TagDialogState | null>(null);
  const [deletingTag, setDeletingTag] = useState<TableTag | null>(null);
  const [hoverDrop, setHoverDrop] = useState<HoverDrop | null>(null);
  const treeRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const schemas = useMemo(() => catalog?.schemas ?? [], [catalog]);
  const visibleSchemas = useMemo(() => schemas.filter((s) => schemaFilter === null || s.name === schemaFilter), [schemas, schemaFilter]);
  const headers = schemaFilter === null && schemas.length > 1;
  const tagMode = mode === "tags" && !keyValue;

  const rows = useMemo(
    () =>
      buildRows({
        schemas: visibleSchemas,
        headers,
        namespaced,
        mode: keyValue ? "az" : mode,
        needle: search,
        tags: tagState,
        expanded,
        expandColumns: columnPreview,
        columnsFor: (table) => columnsCache[`${id}:${tableKey(table)}`],
      }),
    [visibleSchemas, headers, namespaced, mode, keyValue, search, tagState, expanded, columnPreview, columnsCache, id],
  );
  const effectiveFocus = useMemo(() => (focusKey !== null && rows.some((r) => r.key === focusKey) ? focusKey : null), [rows, focusKey]);

  // The schemas a new tag may live in; a bare "New tag" targets the first.
  const tagScopes = useMemo<TagScopeChoice[]>(() => {
    const seen = new Set<string>();
    const out: TagScopeChoice[] = [];
    for (const schema of visibleSchemas) {
      const scope = schemaScope(schema, namespaced);
      if (seen.has(scope)) continue;
      seen.add(scope);
      out.push({ scope, label: schema.name });
    }
    return out;
  }, [visibleSchemas, namespaced]);

  const live = useRef<Live | null>(null);
  useLayoutEffect(() => {
    live.current = { id, connection, database, speaksSql, keyValue, noun, columnPreview, profile, rows, tagState, columnsCache, tagMode, menu };
  });

  const handlers = useMemo<PanelHandlers>(() => {
    const L = (): Live => {
      const current = live.current;
      if (current === null) throw new Error("The sidebar handlers ran before the panel mounted.");
      return current;
    };
    const ws = () => useWorkspace.getState();
    const refOf = (table: TableInfo): TableRef => ({ schema: table.schema, name: table.name });
    const fail = (raw: Parameters<typeof normalizeError>[0]) => {
      ws().showError(normalizeError(raw));
    };
    const copy = (text: string, done: string) => {
      navigator.clipboard.writeText(text).then(() => { ws().showInfo(done); }, fail);
    };

    const toggleColumns = (row: TableRow) => {
      const l = L();
      if (!row.expandable) return;
      setExpanded((prev) => {
        const next = new Set(prev);
        if (next.has(row.ref)) next.delete(row.ref);
        else next.add(row.ref);
        return next;
      });
      if (!row.expanded && l.columnsCache[`${l.id}:${row.ref}`] === undefined) {
        ws().loadColumns(l.id, refOf(row.table)).catch(fail);
      }
    };

    const toggleGroup = (row: GroupRow) => {
      const l = L();
      void useTableTags.getState().setCollapsed(l.id, l.database, row.scope, row.tag?.id ?? null, !row.collapsed);
    };

    const openTable = (row: TableRow, newTab: boolean) => {
      const l = L();
      if (newTab) ws().openTableInNewTab(l.id, refOf(row.table));
      else ws().openTable(l.id, refOf(row.table));
    };

    // WHAT:  Forgets the cached columns of one table and asks the server again,
    //        past the persisted schema cache (`force`).
    const refreshColumns = (table: TableInfo) => {
      const l = L();
      const ref = refOf(table);
      ws().forgetColumns(l.id, ref);
      ipc("load_columns", { connectionId: l.id, table: ref, force: true }).then((columns) => {
        useWorkspace.setState((s) => ({ columnsCache: { ...s.columnsCache, [`${l.id}:${tableKey(ref)}`]: columns } }));
      }, fail);
    };

    // WHAT:  The real DDL when the engine can give it, the column sketch otherwise.
    // WHY:   The sketch is only the columns; the DDL carries defaults, keys and
    //        indexes. A view without a definition has no honest sketch to fall back on.
    const copyDdl = (table: TableInfo) => {
      const l = L();
      const ref = refOf(table);
      void (async () => {
        try {
          const ddl = await ipc("load_ddl", { connectionId: l.id, table: ref });
          if (ddl !== null && ddl.trim() !== "") {
            copy(ddl, table.kind === "view" ? "View definition copied." : "Table DDL copied.");
            return;
          }
        } catch {
          // fall through to the sketch
        }
        if (table.kind === "view") {
          ws().showError("This engine could not give the view's definition.");
          return;
        }
        try {
          const columns = await ws().loadColumns(l.id, ref);
          copy(createTableSketch(l.connection.engine, ref, columns), "Copied a CREATE TABLE sketch of the columns (the engine could not give its own DDL).");
        } catch (raw) {
          fail(raw);
        }
      })();
    };

    const tableMenuContext = (row: TableRow) => {
      const l = L();
      const state = useTableTags.getState().states[tagScopeKey(l.id, l.database)];
      return {
        table: row.table,
        noun: l.noun,
        speaksSql: l.speaksSql,
        columnPreview: l.columnPreview,
        expanded: row.expanded,
        profile: l.profile,
        pinned: ws().isPinned("table", tableItemId(l.id, refOf(row.table))),
        tagging: !l.keyValue,
        scopeTags: tagsForScope(state, row.scope),
        taggedWith: new Set(tagsOfTable(state, row.scope, row.table.name).map((t) => t.id)),
      };
    };

    const runTableAction = (row: TableRow, command: string) => {
      const l = L();
      const table = row.table;
      const ref = refOf(table);
      const quoted = qualifiedIdentifier(l.connection.engine, ref);
      const form = tableAction(command, table);
      if (form !== null) {
        setAction(form);
        return;
      }
      if (command.startsWith("tag:")) {
        const tagId = command.slice(4);
        const state = useTableTags.getState().states[tagScopeKey(l.id, l.database)];
        const has = tagsOfTable(state, row.scope, table.name).some((t) => t.id === tagId);
        void useTableTags.getState().tagTable(l.id, l.database, tagId, table.name, !has);
        return;
      }
      switch (command) {
        case "open":
          openTable(row, false);
          break;
        case "open-new":
          openTable(row, true);
          break;
        case "columns":
          toggleColumns(row);
          break;
        case "refresh-columns":
          refreshColumns(table);
          break;
        case "select":
          ws().openQuery(l.id, `SELECT *\nFROM ${quoted}\nLIMIT 100;`, table.name);
          break;
        case "count":
          ws().openQuery(l.id, `SELECT count(*) FROM ${quoted};`, `count ${table.name}`);
          break;
        case "export":
          ws().openTransfer(l.id);
          break;
        case "compare-schema":
          ws().openSchemaCompare(l.id, table.schema);
          break;
        case "compare-data":
          ws().openDataCompare(l.id, ref);
          break;
        case "copy-name":
          copy(table.name, "Name copied.");
          break;
        case "copy-qualified":
          copy(table.schema === null ? table.name : `${table.schema}.${table.name}`, "Qualified name copied.");
          break;
        case "copy-schema":
          copyDdl(table);
          break;
        case "refresh":
          void ws().loadCatalog(l.id, true);
          break;
        case "pin":
          void ws().pinItem(tableEntity(l.id, ref));
          break;
        case "unpin":
          void ws().unpinItem("table", tableItemId(l.id, ref));
          break;
        case "ai-dashboard":
          // The chat turns the `@table` token into a mention, so the model is handed its definition.
          ws().openChat(l.id, dashboardForTablePrompt(table.schema === null ? table.name : `${table.schema}.${table.name}`));
          break;
        case "tag-new":
          setTagDialog({ kind: "create", scope: row.scope, thenTable: table.name });
          break;
        case "tag-clear":
          void useTableTags.getState().untagTable(l.id, l.database, row.scope, table.name);
          break;
        default:
          break;
      }
    };

    const runSchemaAction = (row: SchemaRow, command: string) => {
      const l = L();
      const form = schemaAction(command, row.name);
      if (form !== null) {
        setAction(form);
        return;
      }
      switch (command) {
        case "only":
          ws().setSchemaFilter(l.id, row.name);
          break;
        case "erd":
          ws().openErd(l.id, row.name);
          break;
        case "compare":
          ws().openSchemaCompare(l.id, row.name);
          break;
        case "copy":
          copy(row.name, "Schema name copied.");
          break;
        case "refresh":
          void ws().loadCatalog(l.id, true);
          break;
        case "tag-new":
          setTagDialog({ kind: "create", scope: row.scope, thenTable: null });
          break;
        default:
          break;
      }
    };

    // What a row's menu shows and does, wherever it is opened from.
    const describeMenu = (row: TreeRow): { build: () => readonly MenuNode[]; select: (command: string) => void } | null => {
      switch (row.kind) {
        case "table":
          return { build: () => tableMenu(tableMenuContext(row)), select: (command) => { runTableAction(row, command); } };
        case "schema":
          return {
            build: () => {
              const l = L();
              return schemaMenu({ name: row.name, speaksSql: l.speaksSql, erd: supportsErd(l.connection.engine), profile: l.profile, tagging: l.tagMode });
            },
            select: (command) => { runSchemaAction(row, command); },
          };
        case "group": {
          const tag = row.tag;
          if (tag === null) return null;
          return {
            build: () => groupMenu(tag),
            select: (command) => {
              if (command === "tag-edit") setTagDialog({ kind: "edit", tag });
              else if (command === "tag-new") setTagDialog({ kind: "create", scope: row.scope, thenTable: null });
              else if (command === "tag-delete") setDeletingTag(tag);
            },
          };
        }
        case "column":
          return {
            build: () => columnMenu(row.column.dataType),
            select: (command) => { copy(command === "name" ? row.column.name : row.column.dataType, command === "name" ? "Column name copied." : "Column type copied."); },
          };
        default:
          return null;
      }
    };

    const rowMenu = (event: ReactMouseEvent, row: TreeRow) => {
      const description = describeMenu(row);
      if (description === null) return;
      setFocusKey(row.key);
      L().menu.openFromEvent(event, description.build, description.select);
    };

    const buttonMenu = (element: Element, row: TreeRow) => {
      const description = describeMenu(row);
      if (description !== null) L().menu.openBelow(element, description.build, description.select);
    };

    const onMenuKey = (row: TreeRow, element: HTMLElement | null) => {
      const description = describeMenu(row);
      if (description === null || element === null) return;
      L().menu.openBelow(element, description.build, description.select);
    };

    const focusRow = (key: string) => {
      setFocusKey(key);
      treeRef.current?.focus({ preventScroll: true });
    };

    // ---- drag: tag groups reorder; tables go to the split pane -------------------
    const groupPointerDown = (event: ReactPointerEvent, row: GroupRow) => {
      const tag = row.tag;
      if (tag === null) return;
      if (event.target instanceof Element && event.target.closest("button") !== null) return;
      const l = L();
      const scopeIds = tagsForScope(l.tagState, row.scope).map((t) => t.id);
      beginPointerDrag(event, {
        payload: tag.id,
        label: tag.name,
        zones: ["tag"],
        onHover: (hit, point) => {
          const scroller = scrollRef.current;
          if (scroller !== null && hit !== null) {
            const box = scroller.getBoundingClientRect();
            if (point.y < box.top + 28) scroller.scrollTop -= 14;
            else if (point.y > box.bottom - 28) scroller.scrollTop += 14;
          }
          setHoverDrop(hit !== null && hit.id !== null && hit.id !== tag.id && scopeIds.includes(hit.id) ? { id: hit.id, placement: hit.placement } : null);
        },
        onDrop: (hit, dragged) => {
          if (hit.id === null) return;
          const next = reorderIds(scopeIds, dragged, hit.id, hit.placement);
          if (next.join() !== scopeIds.join()) void useTableTags.getState().reorder(l.id, l.database, row.scope, next);
        },
        onEnd: () => { setHoverDrop(null); },
      });
    };

    // WHAT:  A table row dragged onto a pane or a tab strip (the shell's pane drag).
    // WHY:   The drop decides where the tab goes; `open` only has to make the tab
    //        the active one. A table that is already open is brought forward so the
    //        drop moves its tab; otherwise it gets a tab of its own (never the
    //        reusable preview tab, which the next click would replace).
    const tablePointerDown = (event: ReactPointerEvent, row: TableRow) => {
      const l = L();
      const ref = refOf(row.table);
      beginPaneDrag(event, {
        kind: "open",
        label: row.table.name,
        icon: row.table.kind === "view" ? "view" : "table",
        open: () => {
          const open = ws().tabs.find((t) => t.kind === "table" && t.connectionId === l.id && tableKey(t.table) === tableKey(ref));
          if (open !== undefined) ws().activateTab(open.id);
          else ws().openTableInNewTab(l.id, ref);
        },
      });
    };

    const onIntent = (intent: KeyIntent) => {
      if (intent.type === "none") return;
      if (intent.type === "focus") {
        setFocusKey(intent.key);
        return;
      }
      const row = L().rows.find((r) => r.key === intent.key);
      if (row === undefined) return;
      if (intent.type === "open") {
        if (row.kind === "table") openTable(row, intent.newTab);
        return;
      }
      if (row.kind === "table") toggleColumns(row);
      else if (row.kind === "group") toggleGroup(row);
    };

    return { focusRow, openTable, toggleColumns, toggleGroup, rowMenu, buttonMenu, tablePointerDown, groupPointerDown, refreshColumns, onIntent, onMenuKey };
  }, []);

  // ---- toolbar ---------------------------------------------------------------------
  const createAllowed = ddlAllowed(profile, speaksSql, "createTable");
  const createSchema = namespaced ? (schemaFilter ?? schemas[0]?.name ?? null) : null;
  const newTable = () => { setAction({ kind: "create-table", schema: createSchema }); };
  const newTag = () => { setTagDialog({ kind: "create", scope: tagScopes[0]?.scope ?? "", thenTable: null }); };

  const databases = useMemo(() => info?.databases ?? [], [info]);
  const currentDb = info?.database ?? "";
  const dbOptions = (databases.length > 0 ? databases : [currentDb]).map((d) => ({ value: d, label: d }));
  const schemaOptions = [{ value: "*", label: "All schemas" }, ...schemas.map((s) => ({ value: s.name, label: s.name }))];
  const namespaceNodes = useMemo(
    () => namespaceMenu({ databases, currentDatabase: currentDb, schemas: namespaced ? schemas.map((s) => s.name) : [], profile }),
    [databases, currentDb, namespaced, schemas, profile],
  );

  const needle = serverSearch ? "" : search.trim();
  const keyTables = useMemo(
    () => visibleSchemas.flatMap((s) => (needle === "" ? s.tables : s.tables.filter((t) => t.name.toLowerCase().includes(needle.toLowerCase())))),
    [visibleSchemas, needle],
  );

  const focusList = () => {
    const first = rows.find(isFocusable);
    treeRef.current?.focus();
    setFocusKey((current) => effectiveFocus ?? current ?? first?.key ?? null);
  };

  const onSearchKey = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusList();
    } else if (event.key === "Escape") {
      setSearch("");
      setSearchOpen(false);
    }
  };

  const changeMode = (next: SortMode) => {
    setModeState(next);
    writeMode(id, next);
  };

  return (
    <aside className="flex h-full w-full min-w-0 flex-col glass-sidebar select-none">
      {/* WHAT:  Header that survives the sidebar's full 180-520px range.
          WHY:   Five actions plus a connection name need ~320px. Below that the
                 row used to overflow and simply cut the last buttons off, which
                 is the one outcome a toolbar must never have.
          HOW:   Container queries, so the breakpoints follow the sidebar's own
                 width rather than the window's: the connection name drops first,
                 then its engine logo, and the three occasional actions fold into
                 an overflow menu — Refresh and Search stay reachable at every
                 width. */}
      <div className="@container drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-1.5 overflow-hidden border-b border-border/40" data-tauri-drag-region>
        <ConnectionSwitcher caption={collectionNoun(engine)} />
        <div className="drag-region h-full min-w-4 flex-1" data-tauri-drag-region />
        <span className="flex shrink-0 items-center gap-0.5">
          <IconButton icon="refresh" label="Refresh schema" onClick={() => void loadCatalog(id, true)} />
          <span className="hidden items-center gap-0.5 @[24rem]:flex">
            <IconButton icon="plus" label="New query" onClick={() => openQuery(id)} />
            <IconButton icon="view" label="ER diagram" disabled={!supportsErd(engine)} onClick={() => openErd(id, schemaFilter)} />
            <IconButton icon="table" label={`New ${noun}`} disabled={!createAllowed} onClick={newTable} />
          </span>
          <span className="flex @[24rem]:hidden">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" aria-label="More actions" className="rounded-md">
                  <Icon name="more" size={15} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => { openQuery(id); }}>
                  <Icon name="plus" size={13} className="text-muted" />
                  New query
                </DropdownMenuItem>
                <DropdownMenuItem disabled={!supportsErd(engine)} onSelect={() => { openErd(id, schemaFilter); }}>
                  <Icon name="view" size={13} className="text-muted" />
                  ER diagram
                </DropdownMenuItem>
                <DropdownMenuItem disabled={!createAllowed} onSelect={newTable}>
                  <Icon name="table" size={13} className="text-muted" />
                  New {noun}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </span>
          {serverSearch ? null : <IconButton icon="search" label="Search tables" active={searchOpen} onClick={() => setSearchOpen((v) => !v)} />}
        </span>
      </div>

      <div className="flex items-center gap-1 px-2.5 py-1.5 text-xs text-muted">
        <AppSelect ariaLabel="Database" value={currentDb} options={dbOptions} plain className="w-auto min-w-0" icon="database" onChange={(db) => void switchDatabase(id, db)} />
        {namespaced && (schemas.length > 1 || (schemas[0] !== undefined && schemas[0].name !== "main")) ? (
          <>
            <span className="px-0.5 text-muted/60">/</span>
            <AppSelect ariaLabel="Schema" value={schemaFilter ?? "*"} options={schemaOptions} plain className="w-auto min-w-0" icon="folder" onChange={(v) => setSchemaFilter(id, v === "*" ? null : v)} />
          </>
        ) : null}
        {namespaceNodes.length > 0 ? (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label="Database and schema actions" className="ml-auto size-5 min-w-5 shrink-0 rounded-md">
                <Icon name="more" size={13} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-56">
              <MenuNodes
                nodes={namespaceNodes}
                onSelect={(command) => {
                  const form = namespaceAction(command);
                  if (form !== null) setAction(form);
                }}
              />
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>

      {keyValue ? null : (
        <div className="flex items-center gap-1.5 px-2.5 pb-1.5">
          <Segmented label="Table list order" value={mode} onChange={changeMode} options={SORT_MODES} className="min-w-0 flex-1" />
          {tagMode ? <IconButton icon="tag" label="New tag" onClick={newTag} /> : null}
        </div>
      )}

      {tagMode && tagState?.tags.length === 0 && needle === "" ? (
        <p className="px-2.5 pb-1.5 text-[11px] leading-snug text-muted">No tags yet. Right-click a table and choose Tag table, or press the tag button to make one.</p>
      ) : null}

      {searchOpen && !serverSearch ? (
        <div className="px-2.5 pb-1.5">
          <SearchInput value={search} onChange={setSearch} onKeyDown={onSearchKey} aria-label="Search tables" placeholder="Search tables…" autoFocus className="glass-input w-full rounded-md text-xs" />
        </div>
      ) : null}

      <Separator className="opacity-50" />

      {connecting === id || !catalog ? (
        <div className="space-y-2 p-2.5">
          <Skeleton className="h-4 w-3/4 rounded-md" />
          <Skeleton className="h-4 w-1/2 rounded-md" />
          <Skeleton className="h-4 w-5/6 rounded-md" />
          <Skeleton className="h-4 w-2/3 rounded-md" />
        </div>
      ) : keyValue ? (
        <ScrollArea className="min-h-0 flex-1 px-1.5 py-1">
          {keyTables.length === 0 && !serverSearch ? (
            <p className="px-2.5 py-2 text-xs text-muted">{needle !== "" ? "No keys match." : "No keys in this database yet."}</p>
          ) : (
            <KeyTree connectionId={id} keys={keyTables} />
          )}
        </ScrollArea>
      ) : rows.length === 0 ? (
        <ScrollArea className="min-h-0 flex-1 px-1.5 py-1">
          <p className="px-2.5 py-2 text-xs text-muted">{needle !== "" ? `No ${noun}s match.` : `No ${noun}s in this database yet. Use New ${noun} above, or create one from a query tab and refresh.`}</p>
        </ScrollArea>
      ) : (
        <TableTreeList
          rows={rows}
          focusKey={effectiveFocus}
          activeRef={activeRef}
          hoverDrop={hoverDrop}
          handlers={handlers}
          onIntent={handlers.onIntent}
          onMenuKey={handlers.onMenuKey}
          treeRef={treeRef}
          scrollRef={scrollRef}
        />
      )}

      {info?.serverVersion ? <div className="truncate border-t border-border/40 px-2.5 py-1 font-mono text-[10px] text-muted/70">{info.serverVersion}</div> : null}
      {menu.node}

      {tagDialog !== null ? (
        <TagDialog
          key={tagDialog.kind === "edit" ? tagDialog.tag.id : "new"}
          connectionId={id}
          database={database}
          scopes={tagDialog.kind === "create" && tagDialog.thenTable !== null ? tagScopes.filter((s) => s.scope === tagDialog.scope) : tagScopes}
          initialScope={tagDialog.kind === "create" ? tagDialog.scope : tagDialog.tag.schema}
          existing={tagState?.tags ?? []}
          {...(tagDialog.kind === "edit" ? { tag: tagDialog.tag } : {})}
          onSaved={(saved) => {
            if (tagDialog.kind === "create" && tagDialog.thenTable !== null) void useTableTags.getState().tagTable(id, database, saved.id, tagDialog.thenTable, true);
          }}
          onClose={() => { setTagDialog(null); }}
        />
      ) : null}
      {deletingTag !== null ? (
        <DeleteTagDialog
          tag={deletingTag}
          tables={tagMemberCount(tagState, deletingTag.id)}
          onConfirm={() => useTableTags.getState().deleteTag(id, database, deletingTag.id)}
          onClose={() => { setDeletingTag(null); }}
        />
      ) : null}
      <SchemaActionHost connectionId={id} action={action} onClose={() => { setAction(null); }} />
    </aside>
  );
}
