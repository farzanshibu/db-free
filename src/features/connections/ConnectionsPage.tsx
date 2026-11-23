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
import { IconButton } from "@/components/global/Button";
import { InfoTip } from "@/components/global/InfoTip";
import { AutoGrid, Page, PageHeader } from "@/components/global/Layout";
import { OverflowMenu, type ToolbarAction } from "@/components/global/Toolbar";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { SearchInput } from "@/components/ui/input";
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

  const moreActions: ToolbarAction[] = [
    { id: "import", label: "Import connections…", icon: "transfer", onSelect: () => { setImporting(true); }, group: "transfer" },
    { id: "export", label: "Export connections…", icon: "download", disabled: saved.length === 0, onSelect: () => { setExporting(true); }, group: "transfer" },
    { id: "organize", label: "Tags and folders…", icon: "tag", onSelect: () => { setManaging(true); }, group: "org" },
    { id: "demo", label: "Open the demo database", icon: "package", onSelect: () => void tryDemo(), group: "org" },
  ];

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col">
      <Page
        header={
          <PageHeader
            title="Connections"
            meta={`${saved.length} saved`}
            tip="Search covers name, host, engine, folder and tag. Right-click a card for more."
            actions={
              <>
                {connections.length > 0 ? (
                  <div className="w-64 min-w-0">
                    <SearchInput value={search} onChange={setSearch} aria-label="Search connections" placeholder="Search…" className="glass-input w-full rounded-md text-xs" />
                  </div>
                ) : null}
                <OverflowMenu items={moreActions} label="More actions" />
                <Button size="sm" onClick={() => openForm()} className="font-semibold liquid-hover">
                  <Icon name="plus" size={14} />
                  New Connection
                </Button>
              </>
            }
          />
        }
      >
        <CredentialBanner />

        {folders.length > 0 ? <FolderChips folders={folders} counts={countsByFolder(connections)} total={saved.length} active={folderFilter} onSelect={setFolderFilter} onManage={() => { setManaging(true); }} /> : null}

        <HomeBoard />
        {connections.length === 0 ? (
          <Card className="w-full glass-card rounded-lg border-border/60">
            <CardContent>
              <EmptyState
                icon="plug"
                title="No connections yet"
                tip="PostgreSQL, MySQL, SQLite, Redis, MongoDB and 60+ more engines. The demo is a sample shop database. Import and export live in the ⋯ menu."
                action={
                  <div className="flex flex-wrap items-center justify-center gap-2">
                    <Button onClick={() => openForm()} className="font-semibold">
                      <Icon name="plus" size={14} />
                      New connection
                    </Button>
                    <Button variant="secondary" pending={demoBusy} onClick={() => void tryDemo()}>
                      <Icon name="package" size={14} />
                      Try the demo
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
      </Page>
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
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by folder">
      {chip(null, "All", total)}
      {folders.map((f) => chip(f.id, f.name, counts[f.id] ?? 0))}
      <IconButton icon="settings" label="Manage folders and tags" onClick={onManage} size={13} className="size-6 rounded-full text-muted" />
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
          <p className="flex items-center gap-1 px-2 text-xs text-muted">
            Empty folder.
            <InfoTip label="Add to this folder">Right-click a connection and choose “Tags and folders…”.</InfoTip>
          </p>
        ) : (
          // WHAT:  Cards flow into as many columns as the window affords.
          // WHY:   One narrow column left most of a desktop window empty, and
          //        the list got longer the more connections someone saved —
          //        exactly the wrong way round.
          <AutoGrid min="lg">{children}</AutoGrid>
        )
      ) : null}
    </section>
  );
}
