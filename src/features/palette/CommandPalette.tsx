// SOT: command-palette, cmd-k, quick-actions, palette-recents, palette-sql-shortcut
import { useMemo, useState, type ReactNode } from "react";
import { tableKey, useActiveConnection, useWorkspace } from "@/stores/workspace";
import { useBackupDialog } from "@/features/backup/useBackupDialog";
import { useShortcut } from "@/stores/useShortcut";
import { Icon, type IconName } from "@/lib/icons";
import { EngineIcon } from "@/components/global/EngineIcon";
import { engineMeta } from "@/lib/engines";
import { TOOL_ORDER, toolMeta, toolsOf } from "@/lib/objects";
import { bestScore, looksLikeSql } from "@/lib/fuzzy";
import { parseJson } from "@/lib/json";
import { readStored, writeStored } from "@/lib/storage";
import { LAYOUT_PRESETS } from "@/lib/layouts";
import { runShellAction } from "@/features/shell/shellActions";
import { ConnectionChip, useConnectionIdentity } from "@/features/shell/TabIdentity";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/ui/kbd";
import { Button } from "@/components/ui/button";
import { Command, CommandDialog, CommandEmpty, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { libraryActions, useLibraryPaletteState } from "./libraryActions";

const EMPTY_SECTIONS: string[] = [];

const RECENTS_KEY = "db-free:palette-recents";
const MAX_RECENTS = 8;
const MAX_RESULTS = 60;
/// Sections that can be huge (every table, every saved query): without a query only the first few show.
const LARGE_SECTIONS = new Set(["tables", "saved_queries", "dashboards", "workflows", "diagrams"]);
const LARGE_SECTION_PREVIEW = 6;
/// The "open what you typed as a query" entry: one-off text, never remembered as a recent.
const SQL_ACTION_ID = "sql:open-as-query";

// WHAT:  The ids of the actions the user ran last, newest first.
// WHY:   The thing you did a minute ago is the thing you want again; showing it
//        before you type saves the search.
function readRecents(): string[] {
  const parsed = parseJson(readStored(RECENTS_KEY) ?? "");
  return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string").slice(0, MAX_RECENTS) : [];
}

interface Action {
  id: string;
  section: string;
  label: string;
  hint?: string;
  icon: IconName;
  /// Replaces `icon` when set (engine logo for connections).
  leading?: ReactNode;
  run: () => void;
}

// WHAT:  ⌘/Ctrl+K palette: switch connection, jump to a table, open any tab, settings.
// WHY:   PRD §5 keyboard-first workflows.
export function CommandPalette() {
  const open = useWorkspace((s) => s.paletteOpen);
  const setOpen = useWorkspace((s) => s.setPaletteOpen);
  const connections = useWorkspace((s) => s.connections);
  const connection = useActiveConnection();
  // The connection the table and query actions below act on: the one of the tab in front.
  const identity = useConnectionIdentity(connection?.id ?? null);
  const catalog = useWorkspace((s) => (connection ? s.catalogs[connection.id] : undefined));
  const savedQueries = useWorkspace((s) => s.savedQueries);
  const library = useLibraryPaletteState();
  const sections = useWorkspace((s) => s.settings?.commandMenuSections ?? EMPTY_SECTIONS);
  const hidden = useWorkspace((s) => s.settings?.commandMenuHiddenSections ?? EMPTY_SECTIONS);
  const store = useWorkspace;
  const [query, setQuery] = useState("");
  const [recents, setRecents] = useState<string[]>(readRecents);

  useShortcut("palette", () => setOpen(!store.getState().paletteOpen));

  const actions = useMemo<Action[]>(() => {
    const s = store.getState();
    const cid = connection?.id ?? null;
    const list: Action[] = [];
    list.push({ id: "new-connection", section: "create", label: "New connection", icon: "plus", run: () => s.goPicker() });
    if (cid) {
      list.push({ id: "new-query", section: "create", label: "New query tab", icon: "terminal", run: () => s.openQuery(cid) });
      list.push({ id: "new-dashboard", section: "create", label: "New dashboard", icon: "columns", run: () => s.setSidebar("dashboards") });
      list.push({ id: "new-workflow", section: "create", label: "New workflow", icon: "play", run: () => s.setSidebar("workflows") });
      list.push({ id: "new-diagram", section: "create", label: "New schema diagram", icon: "view", run: () => s.setSidebar("diagrams") });
      list.push({ id: "history", section: "navigation", label: "Query history", icon: "history", run: () => s.openHistory(cid) });
      list.push({ id: "transfer", section: "navigation", label: "Export / Import", icon: "download", run: () => s.openTransfer(cid) });
      list.push({ id: "backup", section: "navigation", label: "Backup / Restore", hint: "pg_dump, mysqldump, mongodump, file copy", icon: "archive", run: () => useBackupDialog.getState().open(cid) });
      list.push({ id: "erd", section: "navigation", label: "ER diagram of current schema", icon: "view", run: () => s.openErd(cid, s.schemaFilter[cid] ?? null) });
      list.push({ id: "chat", section: "navigation", label: "Chat with database", hint: "Ask questions, explore schema, generate queries", icon: "braces", run: () => s.openChat(cid) });
      list.push({ id: "tables", section: "navigation", label: "Tables sidebar", icon: "table", run: () => s.setSidebar("tables") });
      list.push({ id: "queries", section: "navigation", label: "Saved queries sidebar", icon: "file", run: () => s.setSidebar("queries") });
      list.push({ id: "objects", section: "navigation", label: "Objects sidebar", hint: "Views, functions, triggers, indexes, streams…", icon: "hierarchy", run: () => s.setSidebar("objects") });
      list.push({ id: "schema-compare", section: "navigation", label: "Compare schemas", hint: "Diff two schemas and generate a migration", icon: "git-branch", run: () => s.openSchemaCompare(cid, s.schemaFilter[cid] ?? null) });
      list.push({ id: "data-compare", section: "navigation", label: "Compare table data", hint: "Diff two tables row by row and generate a sync script", icon: "exchange", run: () => s.openDataCompare(cid, null) });
      list.push({ id: "admin", section: "navigation", label: "Server admin", hint: "Overview, sessions, users, settings", icon: "server", run: () => s.openAdmin(cid) });
      const engine = s.connections.find((c) => c.id === cid)?.engine;
      const tools = s.sessionInfos[cid]?.tools ?? (engine ? toolsOf(engine) : []);
      for (const tool of TOOL_ORDER) {
        if (!tools.includes(tool) || tool === "stats" || tool === "erd" || tool === "key_browser") continue;
        const meta = toolMeta(tool);
        list.push({ id: `tool:${tool}`, section: "navigation", label: meta.label, hint: meta.hint, icon: meta.icon, run: () => s.openTool(cid, tool) });
      }
      list.push({
        id: "open-sql-file",
        section: "create",
        label: "Open SQL file…",
        hint: "Load a .sql script into a new query tab",
        icon: "file",
        run: () => {
          void import("@/features/editor/openSqlFile").then((m) => m.openSqlFileInNewTab(cid));
        },
      });
      for (const preset of LAYOUT_PRESETS) {
        list.push({ id: `layout:${preset.id}`, section: "navigation", label: `Layout: ${preset.label}`, hint: preset.description, icon: "sidebar", run: () => s.applyLayout(preset.id) });
      }
      list.push({ id: "toggle-sidebar", section: "navigation", label: "Toggle sidebar", icon: "sidebar", run: () => s.toggleSidebar() });
      list.push({ id: "toggle-inspector", section: "navigation", label: "Toggle record inspector", icon: "inspector", run: () => s.toggleInspector() });
    }
    list.push(...libraryActions(library, cid));
    list.push({ id: "new-window", section: "create", label: "New window", icon: "window", run: () => { runShellAction("new-window"); } });
    list.push({ id: "zoom-in", section: "settings", label: "Zoom in", icon: "expand", run: () => { runShellAction("zoom-in"); } });
    list.push({ id: "zoom-out", section: "settings", label: "Zoom out", icon: "collapse", run: () => { runShellAction("zoom-out"); } });
    list.push({ id: "zoom-reset", section: "settings", label: "Actual size", hint: "Reset the zoom to 100%", icon: "refresh", run: () => { runShellAction("zoom-reset"); } });
    list.push({ id: "whats-new", section: "settings", label: "What's new", hint: "Changelog and release notes", icon: "sparkles", run: () => { runShellAction("whats-new"); } });
    list.push({ id: "check-updates", section: "settings", label: "Check for updates", icon: "download", run: () => { runShellAction("check-updates"); } });
    list.push({ id: "about", section: "settings", label: "About DB Free", hint: "Version, platform, log folder", icon: "info", run: () => { runShellAction("about"); } });
    list.push({ id: "connections", section: "navigation", label: "Connections", icon: "plug", run: () => s.goConnections() });
    list.push({ id: "settings", section: "settings", label: "Settings", icon: "settings", run: () => s.goSettings() });
    list.push({ id: "theme", section: "settings", label: "Theme", hint: "Dark, light or system", icon: "eye", run: () => s.goSettingsSection("themes") });
    list.push({ id: "shortcuts", section: "settings", label: "Keyboard shortcuts", hint: "Rebind keys, keybindings, hotkeys", icon: "hash", run: () => s.goSettingsSection("shortcuts") });
    list.push({ id: "capabilities", section: "settings", label: "Engine capabilities", hint: "Feature × engine matrix", icon: "grid", run: () => s.goCapabilities() });
    for (const c of connections) {
      list.push({ id: `conn:${c.id}`, section: "connections", label: c.name, hint: engineMeta(c.engine).label, icon: "database", leading: <EngineIcon engine={c.engine} size={14} />, run: () => s.selectConnection(c.id) });
    }
    if (cid && catalog) {
      for (const schema of catalog.schemas) {
        for (const t of schema.tables) {
          const ref = { schema: t.schema, name: t.name };
          list.push({ id: `table:${tableKey(ref)}`, section: "tables", label: tableKey(ref), hint: t.kind, icon: "table", run: () => s.openTable(cid, ref) });
        }
      }
    }
    if (cid) {
      for (const q of savedQueries) {
        list.push({ id: `saved:${q.id}`, section: "saved_queries", label: q.name, hint: q.sql.slice(0, 60), icon: "file", run: () => s.openQuery(cid, q.sql, q.name) });
      }
    }
    // Sections the user hid in Settings are left out entirely, not just collapsed.
    return list.filter((a) => !hidden.includes(a.section));
  }, [store, connection, connections, catalog, savedQueries, hidden, library]);

  const needle = query.trim();
  const visible = useMemo<Action[]>(() => {
    const order = new Map(sections.map((name, i) => [name, i]));
    const bySection = (a: Action, b: Action) => (order.get(a.section) ?? 99) - (order.get(b.section) ?? 99);
    if (needle.length === 0) {
      const seen = new Set<string>();
      const recent: Action[] = recents.flatMap((id) => {
        const found = actions.find((a) => a.id === id);
        if (found === undefined || seen.has(id)) return [];
        seen.add(id);
        return [{ ...found, section: "recent" }];
      });
      const counts = new Map<string, number>();
      const rest = [...actions].sort(bySection).filter((a) => {
        if (!LARGE_SECTIONS.has(a.section)) return true;
        const n = (counts.get(a.section) ?? 0) + 1;
        counts.set(a.section, n);
        return n <= LARGE_SECTION_PREVIEW;
      });
      return [...recent.map((a) => ({ ...a, id: `recent:${a.id}` })), ...rest].slice(0, MAX_RESULTS);
    }
    // Text that reads as a statement is offered first, as a query to open.
    const cid = connection?.id ?? null;
    const shortcut: Action[] =
      cid !== null && looksLikeSql(needle)
        ? [{ id: SQL_ACTION_ID, section: "query", label: "Open as new query", hint: needle.replace(/\s+/g, " ").slice(0, 80), icon: "terminal", run: () => store.getState().openQuery(cid, needle) }]
        : [];
    const scored = actions.flatMap((a) => {
      const score = bestScore(needle, a.label, a.hint);
      return score === null ? [] : [{ a, score }];
    });
    scored.sort((x, y) => bySection(x.a, y.a) || y.score - x.score);
    return [...shortcut, ...scored.map((x) => x.a)].slice(0, MAX_RESULTS);
  }, [actions, needle, recents, sections, connection, store]);

  const choose = (a: Action) => {
    setOpen(false);
    setQuery("");
    // Only real, repeatable actions are remembered; a recent re-runs under its own id.
    if (a.id !== SQL_ACTION_ID) {
      const id = a.id.startsWith("recent:") ? a.id.slice("recent:".length) : a.id;
      const next = [id, ...recents.filter((r) => r !== id)].slice(0, MAX_RECENTS);
      setRecents(next);
      writeStored(RECENTS_KEY, JSON.stringify(next));
    }
    a.run();
  };

  return (
    <CommandDialog open={open} onOpenChange={setOpen} title="Command palette" description="Search commands, tables and saved queries">
      {/* The palette does its own filtering and section ordering, so cmdk's is
          off: it would re-rank the list by fuzzy score and lose the order the
          user chose in Settings. cmdk still owns the roving selection. */}
      <Command shouldFilter={false} className="p-2">
        <CommandInput
          value={query}
          onValueChange={setQuery}
          placeholder="Search commands, tables, queries…"
          autoFocus
        />
        <CommandList className="mt-1.5">
          <CommandEmpty>No matching command, table or saved query.</CommandEmpty>
          {visible.map((a) => (
            <CommandItem
              key={a.id}
              value={a.id}
              onSelect={() => {
                choose(a);
              }}
            >
              {a.leading ? <span className="flex shrink-0 items-center">{a.leading}</span> : <Icon name={a.icon} size={14} className="shrink-0 text-accent" />}
              <span className="truncate font-medium text-foreground">{a.label}</span>
              {a.hint !== undefined ? <span className="truncate font-mono text-[10.5px] text-muted/80">{a.hint}</span> : null}
              <Badge size="sm" variant="soft" className="ml-auto text-[9.5px] font-medium tracking-wider uppercase">
                {a.section.replace("_", " ")}
              </Badge>
            </CommandItem>
          ))}
        </CommandList>
        <div className="mt-2 flex items-center justify-between border-t border-border/40 px-2 pt-1.5 text-[11px] text-muted">
          <div className="flex items-center gap-2">
            <span className="flex items-center gap-1">
              <Kbd>↵</Kbd> select
            </span>
            <span className="flex items-center gap-1">
              <Kbd>Esc</Kbd> close
            </span>
            {identity !== null ? (
              <span className="flex items-center gap-1">
                acting on <ConnectionChip identity={identity} />
              </span>
            ) : null}
          </div>
          <span className="flex items-center gap-2">
            {recents.length > 0 && needle.length === 0 ? (
              <Button
                size="sm"
                variant="ghost"
                className="h-5 px-1.5 text-[10px]"
                onClick={() => {
                  setRecents([]);
                  writeStored(RECENTS_KEY, "[]");
                }}
              >
                Clear recents
              </Button>
            ) : null}
            <span className="font-mono text-[10px] text-muted/60">{visible.length} results</span>
          </span>
        </div>
      </Command>
    </CommandDialog>
  );
}
