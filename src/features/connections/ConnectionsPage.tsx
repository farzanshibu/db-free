// SOT: connections-page, connection-list, home-screen, connection-folders-view
import { useState } from "react";
import type { ConnectionSummary } from "@/lib/bindings";
import { engineMeta } from "@/lib/engines";
import { connectionColorMeta, connectionTarget, folderNames, groupConnections, matchesConnection, type ConnectionGroup } from "@/lib/connectionGroups";
import { cn } from "@/lib/cn";
import { parseJson } from "@/lib/json";
import { useWorkspace } from "@/stores/workspace";
import { useBackupDialog } from "@/features/backup/useBackupDialog";
import { IconButton } from "@/components/global/Button";
import { useContextMenu } from "@/components/global/ContextMenu";
import { EnvBadge, EnvDot } from "@/components/global/Badge";
import { EmptyState } from "@/components/global/EmptyState";
import { Field } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";

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
// HOW:   Search narrows by name / host / engine; favourites come first, the rest
//        sit under collapsible folder headers (see groupConnections).
export function ConnectionsPage() {
  const connections = useWorkspace((s) => s.connections);
  const openForm = useWorkspace((s) => s.openForm);
  const organize = useWorkspace((s) => s.organizeConnection);
  const [search, setSearch] = useState("");
  const [collapsed, setCollapsed] = useState<string[]>(loadCollapsed);
  const [moving, setMoving] = useState<ConnectionSummary | null>(null);
  const menu = useContextMenu();

  const needle = search.trim();
  const groups = groupConnections(connections.filter((c) => matchesConnection(c, needle)));
  const toggle = (key: string) =>
    setCollapsed((prev) => {
      const next = prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key];
      saveCollapsed(next);
      return next;
    });

  return (
    <div className="grid-bg flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region h-11 shrink-0" data-tauri-drag-region />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto w-full max-w-6xl px-6 pb-12">
          <div className="mb-6 flex items-center justify-between gap-4">
            <div className="flex items-center gap-2.5">
              <h1 className="text-xl font-bold tracking-tight text-foreground">Connections</h1>
              <Badge size="sm" variant="soft" color="accent" className="font-medium">
                {connections.length} saved
              </Badge>
            </div>
            <div className="flex min-w-0 items-center gap-2">
              {connections.length > 0 ? (
                <div className="w-64 min-w-0">
                  <SearchInput value={search} onChange={setSearch} aria-label="Search connections" placeholder="Search name, host, engine…" className="glass-input h-8 w-full rounded-lg text-xs" />
                </div>
              ) : null}
              <Button onClick={() => openForm()} className="font-semibold liquid-hover">
                <Icon name="plus" size={14} />
                New Connection
              </Button>
            </div>
          </div>

        {connections.length === 0 ? (
          <Card className="w-full glass-card rounded-2xl border-border/60">
            <CardContent>
              <EmptyState
                icon="plug"
                title="No connections yet"
                body="Add a PostgreSQL, SQLite, MySQL, Redis, or MongoDB connection to start browsing tables and running queries."
                action={<Button onClick={() => openForm()} className="font-semibold">Add connection</Button>}
              />
            </CardContent>
          </Card>
        ) : groups.length === 0 ? (
          <p className="px-1 py-6 text-sm text-muted">No connection matches “{needle}”.</p>
        ) : (
          <div className="flex flex-col gap-5">
            {groups.map((group) => (
              <GroupSection
                key={group.key}
                group={group}
                // A search shows every match: collapsing would hide what was asked for.
                open={needle.length > 0 || !collapsed.includes(group.key)}
                onToggle={() => toggle(group.key)}
                menu={menu}
                onMove={setMoving}
              />
            ))}
            {menu.node}
          </div>
        )}
        </div>
      </ScrollArea>
      {moving ? (
        <MoveToFolderDialog
          connection={moving}
          folders={folderNames(connections)}
          onClose={() => setMoving(null)}
          onMove={(folder) => {
            void organize(moving.id, { folder });
            setMoving(null);
          }}
        />
      ) : null}
    </div>
  );
}

function GroupSection({ group, open, onToggle, menu, onMove }: { group: ConnectionGroup; open: boolean; onToggle: () => void; menu: ReturnType<typeof useContextMenu>; onMove: (c: ConnectionSummary) => void }) {
  return (
    <section className="flex flex-col gap-2.5" aria-label={group.label}>
      <Button
        variant="ghost"
        size="sm"
        onClick={onToggle}
        aria-expanded={open}
        className="h-7 w-fit gap-1.5 rounded-md px-1.5 text-xs font-semibold uppercase tracking-wide text-muted hover:text-foreground"
      >
        <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
        <Icon name={group.kind === "favorites" ? "star" : group.kind === "folder" ? "folder" : "plug"} size={12} className={group.kind === "favorites" ? "text-warning" : ""} />
        {group.label}
        <span className="font-normal normal-case tabular-nums opacity-70">{group.items.length}</span>
      </Button>
      {open ? (
        // WHAT:  Cards flow into as many columns as the window affords.
        // WHY:   One narrow column left most of a desktop window empty, and
        //        the list got longer the more connections someone saved —
        //        exactly the wrong way round.
        <div className="grid gap-2.5 md:grid-cols-2 2xl:grid-cols-3">
          {group.items.map((c) => <ConnectionCard key={c.id} connection={c} menu={menu} onMove={onMove} />)}
        </div>
      ) : null}
    </section>
  );
}

function ConnectionCard({ connection: c, menu, onMove }: { connection: ConnectionSummary; menu: ReturnType<typeof useContextMenu>; onMove: (c: ConnectionSummary) => void }) {
  const live = useWorkspace((s) => s.sessions.includes(c.id));
  const connecting = useWorkspace((s) => s.connecting === c.id);
  const select = useWorkspace((s) => s.selectConnection);
  const openForm = useWorkspace((s) => s.openForm);
  const disconnect = useWorkspace((s) => s.disconnect);
  const deleteConnection = useWorkspace((s) => s.deleteConnection);
  const organize = useWorkspace((s) => s.organizeConnection);
  const showInfo = useWorkspace((s) => s.showInfo);
  const openBackup = useBackupDialog((s) => s.open);
  const meta = engineMeta(c.engine);
  const target = connectionTarget(c);

  return (
    <Card
      className="group relative flex min-w-0 flex-row items-center gap-3.5 overflow-hidden rounded-xl border-border/40 glass-card px-4 py-3.5 glass-card-hover"
      onContextMenu={(e) =>
        menu.open(
          e,
          [
            { id: "open", label: live ? "Open" : "Connect", icon: "plug" },
            { id: "disconnect", label: "Disconnect", icon: "x", disabled: !live },
            { id: "favorite", label: c.favorite ? "Remove from favourites" : "Add to favourites", icon: "star", group: "organize" },
            { id: "move", label: "Move to folder…", icon: "folder", group: "organize" },
            { id: "edit", label: "Edit connection…", icon: "pencil", group: "edit" },
            { id: "copy-target", label: "Copy host / file", icon: "copy", group: "edit", disabled: target.length === 0 },
            { id: "backup", label: "Backup / Restore…", icon: "archive", group: "edit" },
            { id: "delete", label: "Delete connection", icon: "trash", danger: true, group: "danger" },
          ],
          (action) => {
            if (action === "open") select(c.id);
            else if (action === "disconnect") void disconnect(c.id);
            else if (action === "favorite") void organize(c.id, { favorite: !c.favorite });
            else if (action === "move") onMove(c);
            else if (action === "edit") openForm(c.id);
            else if (action === "copy-target") { void navigator.clipboard.writeText(target); showInfo("Connection target copied."); }
            else if (action === "delete") void deleteConnection(c.id);
            else if (action === "backup") openBackup(c.id);
          },
        )
      }
    >
      {c.color !== null ? <span aria-hidden className={cn("absolute inset-y-0 left-0 w-1", connectionColorMeta(c.color).fill)} /> : null}
      <CardContent className="flex w-full min-w-0 flex-row items-center gap-3.5 p-0">
        <span className="flex size-10 items-center justify-center overflow-hidden rounded-xl bg-surface-tertiary/70 text-accent shadow-xs border border-border/40 shrink-0">
          <EngineIcon engine={c.engine} size={26} />
        </span>
        {/* `shrink` is deliberate: the primitive ships `shrink-0` so
            buttons in a toolbar keep their size, but this one is the
            card's flexible column — without it the name and target
            keep their full width and the Connect button lands on top
            of them once the grid narrows. */}
        <Button
          variant="ghost"
          onClick={() => select(c.id)}
          className="h-auto min-w-0 flex-1 shrink flex-col items-start justify-start bg-transparent p-0 text-left hover:bg-transparent"
        >
          <span className="flex w-full min-w-0 items-center gap-2">
            <EnvDot environment={c.environment} live={live} />
            <span className="min-w-0 truncate text-[14px] font-semibold tracking-tight text-foreground">{c.name}</span>
            {c.favorite ? <Icon name="star" size={11} className="shrink-0 text-warning" /> : null}
            <EnvBadge environment={c.environment} readOnly={c.readOnly} />
          </span>
          <span className="mt-1 flex w-full min-w-0 items-center gap-2 text-xs text-muted">
            <span className="shrink-0 rounded bg-surface-secondary/70 px-1.5 py-0.5 text-[10px] font-medium text-foreground/80">{meta.label}</span>
            <span className="min-w-0 truncate font-mono text-[11px] opacity-80">{target}</span>
            {live ? <span className="shrink-0 text-[11px] font-medium text-success">● connected</span> : null}
          </span>
        </Button>
        {connecting ? <Spinner size="sm" /> : null}
        <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
          <IconButton icon="star" label={c.favorite ? "Remove from favourites" : "Add to favourites"} active={c.favorite} onClick={() => void organize(c.id, { favorite: !c.favorite })} />
          <IconButton icon="pencil" label="Edit" onClick={() => openForm(c.id)} />
          {live ? <IconButton icon="x" label="Disconnect" onClick={() => void disconnect(c.id)} /> : null}
        </div>
        <Button
          size="sm"
          variant={live ? "secondary" : "primary"}
          className={cn("shrink-0 rounded-lg font-medium liquid-hover", live ? "glass-pill text-foreground" : "bg-accent text-accent-foreground")}
          onClick={() => select(c.id)}
        >
          {live ? "Open" : "Connect"}
        </Button>
      </CardContent>
    </Card>
  );
}

// WHAT:  "Move to folder…": type a folder or pick an existing one; blank = none.
function MoveToFolderDialog({ connection, folders, onClose, onMove }: { connection: ConnectionSummary; folders: string[]; onClose: () => void; onMove: (folder: string | null) => void }) {
  const [folder, setFolder] = useState(connection.folder ?? "");
  const submit = () => onMove(folder.trim().length > 0 ? folder.trim() : null);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>Move “{connection.name}” to folder</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          <Field label="Folder" value={folder} onChange={setFolder} placeholder="Clients / Acme" description="Leave empty to take it out of its folder." autoFocus />
          {folders.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {folders.map((name) => (
                <Button key={name} variant={name === folder.trim() ? "secondary" : "tertiary"} size="sm" onClick={() => setFolder(name)} className="h-7 rounded-md text-xs">
                  <Icon name="folder" size={12} />
                  {name}
                </Button>
              ))}
            </div>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>Cancel</Button>
          <Button onClick={submit}>Move</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
