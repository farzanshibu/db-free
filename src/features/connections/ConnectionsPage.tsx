// SOT: connections-page, connection-list, home-screen
import { engineMeta } from "@/lib/engines";
import { cn } from "@/lib/cn";
import { useWorkspace } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { useContextMenu } from "@/components/global/ContextMenu";
import { EnvBadge, EnvDot } from "@/components/global/Badge";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon } from "@/lib/icons";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";

// WHAT:  Home: saved connections. Click a row to connect and enter the workspace.
export function ConnectionsPage() {
  const connections = useWorkspace((s) => s.connections);
  const sessions = useWorkspace((s) => s.sessions);
  const connecting = useWorkspace((s) => s.connecting);
  const select = useWorkspace((s) => s.selectConnection);
  const openForm = useWorkspace((s) => s.openForm);
  const disconnect = useWorkspace((s) => s.disconnect);
  const deleteConnection = useWorkspace((s) => s.deleteConnection);
  const showInfo = useWorkspace((s) => s.showInfo);
  const menu = useContextMenu();

  return (
    <div className="grid-bg flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region h-11 shrink-0" data-tauri-drag-region />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto w-full max-w-6xl px-6 pb-12">
          <div className="mb-6 flex items-center justify-between">
            <div className="flex items-center gap-2.5">
              <h1 className="text-xl font-bold tracking-tight text-foreground">Connections</h1>
              <Badge size="sm" variant="soft" color="accent" className="font-medium">
                {connections.length} saved
              </Badge>
            </div>
            <Button onClick={() => openForm()} className="font-semibold liquid-hover">
              <Icon name="plus" size={14} />
              New Connection
            </Button>
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
        ) : (
          // WHAT:  Cards flow into as many columns as the window affords.
          // WHY:   One narrow column left most of a desktop window empty, and
          //        the list got longer the more connections someone saved —
          //        exactly the wrong way round.
          <div className="grid gap-2.5 md:grid-cols-2 2xl:grid-cols-3">
            {connections.map((c) => {
              const live = sessions.includes(c.id);
              const meta = engineMeta(c.engine);
              const target = (meta.form === "file") ? (c.filePath ?? "") : `${c.host ?? ""}${c.port !== null ? `:${c.port}` : ""}${c.database ? `/${c.database}` : ""}`;
              return (
                <Card
                  key={c.id}
                  className="group relative flex min-w-0 flex-row items-center gap-3.5 overflow-hidden rounded-xl border-border/40 glass-card px-4 py-3.5 glass-card-hover"
                  onContextMenu={(e) =>
                    menu.open(
                      e,
                      [
                        { id: "open", label: live ? "Open" : "Connect", icon: "plug" },
                        { id: "disconnect", label: "Disconnect", icon: "x", disabled: !live },
                        { id: "edit", label: "Edit connection…", icon: "pencil", group: "edit" },
                        { id: "copy-target", label: "Copy host / file", icon: "copy", group: "edit", disabled: target.length === 0 },
                        { id: "delete", label: "Delete connection", icon: "trash", danger: true, group: "danger" },
                      ],
                      (action) => {
                        if (action === "open") select(c.id);
                        else if (action === "disconnect") void disconnect(c.id);
                        else if (action === "edit") openForm(c.id);
                        else if (action === "copy-target") { void navigator.clipboard.writeText(target); showInfo("Connection target copied."); }
                        else if (action === "delete") void deleteConnection(c.id);
                      },
                    )
                  }
                >
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
                        <EnvBadge environment={c.environment} readOnly={c.readOnly} />
                      </span>
                      <span className="mt-1 flex w-full min-w-0 items-center gap-2 text-xs text-muted">
                        <span className="shrink-0 rounded bg-surface-secondary/70 px-1.5 py-0.5 text-[10px] font-medium text-foreground/80">{meta.label}</span>
                        <span className="min-w-0 truncate font-mono text-[11px] opacity-80">{target}</span>
                        {live ? <span className="shrink-0 text-[11px] font-medium text-success">● connected</span> : null}
                      </span>
                    </Button>
                    {connecting === c.id ? <Spinner size="sm" /> : null}
                    <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
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
            })}
            {menu.node}
          </div>
        )}
        </div>
      </ScrollArea>
    </div>
  );
}
