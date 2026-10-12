// SOT: connections-page, connection-list, home-screen, connection-folders-view
import { useEffect, useState, type ReactNode } from "react";
import type { ConnectionFolder, ConnectionSummary } from "@/lib/bindings";
import { groupConnections, matchesConnection, type ConnectionGroup } from "@/lib/connectionGroups";
import { ipc, normalizeError } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { parseJson } from "@/lib/json";
import { useWorkspace } from "@/stores/workspace";
import { HomeBoard } from "@/features/home/HomeBoard";
import { useContextMenu } from "@/components/global/ContextMenu";
import { ConfirmDialog } from "@/components/global/ConfirmDialog";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ConnectionCard } from "./ConnectionCard";
import { ConnectionFailedDialog } from "./ConnectionFailedDialog";
import { ExportConnectionsDialog, ImportConnectionsDialog } from "./ConnectionTransferDialogs";
import { CredentialBanner } from "./CredentialBanner";
import { ManageOrgDialog } from "./ManageOrgDialog";
import { OrganizeDialog } from "./OrganizeDialog";
import { useConnectionOrg } from "./useConnectionOrg";

const COLLAPSED_KEY = "db-free:connection-groups-collapsed";

// WHAT:  Which folder headers are collapsed, remembered per machine.
function loadCollapsed(): string[] {
  try {
    const parsed = parseJson(localStorage.getItem(COLLAPSED_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === "string") : [];
  } catch {
    return [];
  }
}

function saveCollapsed(keys: readonly string[]) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify(keys));
  } catch {
    // Storage unavailable: collapse state is a convenience only.
  }
}

// WHAT:  Home: saved connections. Click a row to connect and enter the workspace.
// WHY:   With forty connections the page has to answer "where is it?" quickly:
//        search covers name, host, engine, folder and tag; folder chips narrow to
//        one folder; tags show on each card.
// HOW:   Favourites come first, then one section per folder (a connection in two
//        folders appears under both), then the rest. Files opened for a single
//        session sit above them as "Opened files" until saved. Deleting asks you
//        to type the connection's name.
export function ConnectionsPage() {
  const connections = useWorkspace((s) => s.connections);
  const openForm = useWorkspace((s) => s.openForm);
  const select = useWorkspace((s) => s.selectConnection);
  const deleteConnection = useWorkspace((s) => s.deleteConnection);
  const refreshConnections = useWorkspace((s) => s.refreshConnections);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const tags = useConnectionOrg((s) => s.tags);
  const folders = useConnectionOrg((s) => s.folders);
  const folderFilter = useConnectionOrg((s) => s.folderFilter);
  const setFolderFilter = useConnectionOrg((s) => s.setFolderFilter);
  const loadOrg = useConnectionOrg((s) => s.load);
  const [search, setSearch] = useState("");
  const [collapsed, setCollapsed] = useState<string[]>(loadCollapsed);
  const [organizing, setOrganizing] = useState<ConnectionSummary | null>(null);
  const [deleting, setDeleting] = useState<ConnectionSummary | null>(null);
  const [removing, setRemoving] = useState(false);
  const [managing, setManaging] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [demoBusy, setDemoBusy] = useState(false);
  const menu = useContextMenu();

  useEffect(() => {
    void loadOrg();
  }, [loadOrg]);

  const needle = search.trim();
  const matching = connections.filter((c) => matchesConnection(c, needle, tags, folders));
  const activeFolder = folders.find((f) => f.id === folderFilter) ?? null;
  const groups: ConnectionGroup[] =
    activeFolder === null
      ? groupConnections(matching, folders)
      : [{ key: `filter:${activeFolder.id}`, label: activeFolder.name, kind: "folder", folderId: activeFolder.id, items: matching.filter((c) => c.extra.folderIds.includes(activeFolder.id)).sort((a, b) => Number(b.favorite) - Number(a.favorite) || a.name.localeCompare(b.name, undefined, { sensitivity: "base" })) }];
  const saved = connections.filter((c) => !c.extra.transient);
  const toggle = (key: string) =>
    setCollapsed((prev) => {
      const next = prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key];
      saveCollapsed(next);
      return next;
    });

  const tryDemo = async () => {
    setDemoBusy(true);
    try {
      const demo = await ipc("create_demo_connection");
      await refreshConnections();
      select(demo.id);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setDemoBusy(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    setRemoving(true);
    try {
      await deleteConnection(deleting.id);
      showInfo(`Deleted "${deleting.name}".`);
      setDeleting(null);
      void loadOrg();
    } catch (raw) {
      setDeleting(null);
      showError(normalizeError(raw));
    } finally {
      setRemoving(false);
    }
  };

  return (
    <div className="grid-bg flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region app-titlebar shrink-0" data-tauri-drag-region />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto w-full max-w-6xl px-4 pb-6">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <h1 className="text-[13px] font-semibold tracking-tight text-foreground">Connections</h1>
              <Badge size="sm" variant="soft" color="accent" className="font-medium">
                {saved.length} saved
              </Badge>
            </div>
            <div className="flex min-w-0 items-center gap-2">
              {connections.length > 0 ? (
                <div className="w-64 min-w-0">
                  <SearchInput value={search} onChange={setSearch} aria-label="Search connections" placeholder="Search name, host, engine, tag…" className="glass-input w-full rounded-md text-xs" />
                </div>
              ) : null}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="secondary" size="icon" aria-label="More actions" className="glass-pill">
                    <Icon name="more" size={14} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="min-w-56">
                  <DropdownMenuItem onSelect={() => { setImporting(true); }}>
                    <Icon name="transfer" />
                    Import connections…
                  </DropdownMenuItem>
                  <DropdownMenuItem disabled={saved.length === 0} onSelect={() => { setExporting(true); }}>
                    <Icon name="download" />
                    Export connections…
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onSelect={() => { setManaging(true); }}>
                    <Icon name="tag" />
                    Tags and folders…
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => void tryDemo()}>
                    <Icon name="package" />
                    Open the demo database
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <Button onClick={() => openForm()} className="font-semibold liquid-hover">
                <Icon name="plus" size={14} />
                New Connection
              </Button>
            </div>
          </div>

          <CredentialBanner />

          {folders.length > 0 ? <FolderChips folders={folders} counts={countsByFolder(connections)} total={saved.length} active={folderFilter} onSelect={setFolderFilter} onManage={() => { setManaging(true); }} /> : null}

          <HomeBoard />
          {connections.length === 0 ? (
            <Card className="w-full glass-card rounded-lg border-border/60">
              <CardContent>
                <EmptyState
                  icon="plug"
                  title="No connections yet"
                  body="Add a PostgreSQL, SQLite, MySQL, Redis, or MongoDB connection to start browsing tables and running queries, or explore a sample shop database first."
                  action={
                    <div className="flex flex-wrap items-center justify-center gap-2">
                      <Button onClick={() => openForm()} className="font-semibold">
                        Add connection
                      </Button>
                      <Button variant="secondary" pending={demoBusy} onClick={() => void tryDemo()}>
                        <Icon name="package" size={14} />
                        Try the demo
                      </Button>
                      <Button variant="tertiary" onClick={() => { setImporting(true); }}>
                        <Icon name="transfer" size={14} />
                        Import…
                      </Button>
                    </div>
                  }
                />
              </CardContent>
            </Card>
          ) : needle.length > 0 && matching.length === 0 ? (
            <p className="px-1 py-2 text-xs text-muted">No connection matches “{needle}”.</p>
          ) : (
            <div className="flex flex-col gap-3">
              {groups
                .filter((g) => g.items.length > 0 || (needle.length === 0 && g.kind === "folder"))
                .map((group) => (
                  <GroupSection
                    key={group.key}
                    group={group}
                    // A search shows every match: collapsing would hide what was asked for.
                    open={needle.length > 0 || !collapsed.includes(group.key)}
                    onToggle={() => { toggle(group.key); }}
                  >
                    {group.items.map((c) => (
                      <ConnectionCard key={c.id} connection={c} tags={tags} menu={menu} onOrganize={setOrganizing} onDelete={setDeleting} onFilterTag={(tag) => { setSearch(tag.name); }} />
                    ))}
                  </GroupSection>
                ))}
              {menu.node}
            </div>
          )}
        </div>
      </ScrollArea>
      {organizing ? <OrganizeDialog connection={organizing} onClose={() => { setOrganizing(null); }} /> : null}
      <ManageOrgDialog open={managing} onOpenChange={setManaging} />
      <ExportConnectionsDialog open={exporting} onOpenChange={setExporting} />
      <ImportConnectionsDialog open={importing} onOpenChange={setImporting} />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => { if (!open) setDeleting(null); }}
        title="Delete connection?"
        description="The connection and its saved credentials are removed. Query history for it goes too."
        confirmLabel="Delete connection"
        requireText={deleting?.name ?? ""}
        pending={removing}
        onConfirm={confirmDelete}
      />
      <ConnectionFailedDialog />
    </div>
  );
}

// WHAT:  How many saved connections sit in each folder, for the chip counts.
function countsByFolder(connections: readonly ConnectionSummary[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const c of connections) {
    if (c.extra.transient) continue;
    for (const id of c.extra.folderIds) counts[id] = (counts[id] ?? 0) + 1;
  }
  return counts;
}

interface FolderChipsProps {
  folders: readonly ConnectionFolder[];
  counts: Record<string, number>;
  total: number;
  active: string | null;
  onSelect: (id: string | null) => void;
  onManage: () => void;
}

// WHAT:  "All | folder | folder…": one chip narrows the list to that folder.
// WHY:   Folders are entities (they can be empty, and a connection can be in
//        several), so filtering by one is a click rather than a search.
function FolderChips({ folders, counts, total, active, onSelect, onManage }: FolderChipsProps) {
  const chip = (id: string | null, label: string, count: number) => (
    <Button
      key={id ?? "all"}
      variant={active === id ? "secondary" : "ghost"}
      size="sm"
      aria-pressed={active === id}
      onClick={() => { onSelect(id); }}
      className={cn("h-6 gap-1.5 rounded-full px-2 text-xs", active === id ? "glass-pill text-foreground" : "text-muted")}
    >
      {id !== null ? <Icon name="folder" size={11} /> : null}
      {label}
      <span className="font-normal tabular-nums opacity-70">{count}</span>
    </Button>
  );
  return (
    <div className="mb-3 flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by folder">
      {chip(null, "All", total)}
      {folders.map((f) => chip(f.id, f.name, counts[f.id] ?? 0))}
      <Button variant="ghost" size="icon-sm" aria-label="Manage folders and tags" title="Manage folders and tags" onClick={onManage} className="size-6 rounded-full text-muted">
        <Icon name="settings" size={13} />
      </Button>
    </div>
  );
}

function GroupSection({ group, open, onToggle, children }: { group: ConnectionGroup; open: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2" aria-label={group.label}>
      <Button
        variant="ghost"
        size="sm"
        onClick={onToggle}
        aria-expanded={open}
        className="h-6 w-fit gap-1.5 rounded-md px-1.5 text-xs font-semibold uppercase tracking-wide text-muted hover:text-foreground"
      >
        <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
        <Icon name={group.kind === "favorites" ? "star" : group.kind === "folder" ? "folder" : group.kind === "transient" ? "file" : "plug"} size={12} className={group.kind === "favorites" ? "text-warning" : ""} />
        {group.label}
        <span className="font-normal normal-case tabular-nums opacity-70">{group.items.length}</span>
      </Button>
      {open ? (
        group.items.length === 0 ? (
          <p className="px-2 text-xs text-muted">Empty folder. Right-click a connection and choose “Tags and folders…” to add it here.</p>
        ) : (
          // WHAT:  Cards flow into as many columns as the window affords.
          // WHY:   One narrow column left most of a desktop window empty, and
          //        the list got longer the more connections someone saved —
          //        exactly the wrong way round.
          <div className="grid gap-2 md:grid-cols-2 2xl:grid-cols-3">{children}</div>
        )
      ) : null}
    </section>
  );
}
