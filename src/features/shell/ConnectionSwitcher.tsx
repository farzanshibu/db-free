// SOT: connection-switcher, quick-connect, sidebar-title
import { Fragment } from "react";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { useBackupDialog } from "@/features/backup/useBackupDialog";
import { engineMeta } from "@/lib/engines";
import { connectionColorMeta, connectionTarget, groupConnections } from "@/lib/connectionGroups";
import { cn } from "@/lib/cn";
import { EngineIcon } from "@/components/global/EngineIcon";
import { EnvDot } from "@/components/global/Badge";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";

// WHAT:  Sidebar title that doubles as a quick connection switcher: pick another
//        saved connection (connects on demand), add one, or open the list.
// HOW:   Same grouping as the connections page: favourites, then folders.
export function ConnectionSwitcher({ caption }: { caption: string }) {
  const connection = useActiveConnection();
  const connections = useWorkspace((s) => s.connections);
  const sessions = useWorkspace((s) => s.sessions);
  const select = useWorkspace((s) => s.selectConnection);
  const goPicker = useWorkspace((s) => s.goPicker);
  const goConnections = useWorkspace((s) => s.goConnections);
  const openBackup = useBackupDialog((s) => s.open);
  if (!connection) return <span className="text-sm font-medium text-foreground">{caption}</span>;
  const groups = groupConnections(connections);
  const labelled = groups.length > 1;

  return (
    <DropdownMenu>
      {/* asChild keeps this the app's own Button rather than nesting one button
          inside another, which is invalid and breaks the trigger's keyboard use. */}
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          // `shrink` overrides the primitive's `shrink-0`: in a narrow sidebar
          // this is the element that must give way, so the icon buttons beside
          // it stay reachable instead of being pushed out of the panel.
          className="h-8 min-w-0 shrink gap-1.5 rounded-lg px-2 text-sm font-semibold text-foreground glass-pill liquid-hover"
          aria-label={`${caption} — switch connection`}
        >
          <EnvDot environment={connection.environment} live={sessions.includes(connection.id)} />
          <EngineIcon engine={connection.engine} size={16} className="shrink-0" />
          <span className="hidden min-w-0 flex-1 truncate @[13rem]:inline" title={connection.name}>{connection.name}</span>
          {/* Part of the trigger, not a chip beside it: the lock describes this
              connection, and at narrow widths a separate badge was an unlabelled
              box sitting next to the engine's brand tile. */}
          {connection.readOnly ? <Icon name="lock" size={11} className="shrink-0 text-muted" /> : null}
          <Icon name="chevron-down" size={12} className="shrink-0 text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="max-h-[70vh] min-w-[320px] overflow-y-auto">
        {groups.map((group, index) => (
          <Fragment key={group.key}>
            {index > 0 ? <DropdownMenuSeparator /> : null}
            <DropdownMenuGroup>
              {labelled ? (
                <DropdownMenuLabel className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted">
                  <Icon name={group.kind === "favorites" ? "star" : group.kind === "folder" ? "folder" : "plug"} size={11} className={group.kind === "favorites" ? "text-warning" : ""} />
                  {group.label}
                </DropdownMenuLabel>
              ) : null}
              {group.items.map((c) => {
                const meta = engineMeta(c.engine);
                const target = connectionTarget(c);
                return (
                  <DropdownMenuItem key={c.id} textValue={`${c.name} ${target}`} onSelect={() => { select(c.id); }} className="relative">
                    {c.color !== null ? <span aria-hidden className={cn("absolute inset-y-1 left-0 w-0.5 rounded-full", connectionColorMeta(c.color).fill)} /> : null}
                    <EnvDot environment={c.environment} live={sessions.includes(c.id)} />
                    <EngineIcon engine={c.engine} size={20} className="shrink-0" />
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate" title={c.name}>{c.name}</span>
                      <span className="truncate font-mono text-[10px] text-muted">
                        {meta.label} · {target}
                      </span>
                    </span>
                    {c.id === connection.id ? <Icon name="check" size={13} className="ml-auto pl-3 text-accent" /> : null}
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuGroup>
          </Fragment>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuItem textValue="New connection" onSelect={() => { goPicker(); }}>
            <Icon name="plus" size={13} className="text-muted" />
            New connection…
          </DropdownMenuItem>
          <DropdownMenuItem textValue="Manage connections" onSelect={() => { goConnections(); }}>
            <Icon name="plug" size={13} className="text-muted" />
            Manage connections…
          </DropdownMenuItem>
          <DropdownMenuItem textValue="Backup / Restore" onSelect={() => { openBackup(connection.id); }}>
            <Icon name="archive" size={13} className="text-muted" />
            Backup / Restore {connection.name}…
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
