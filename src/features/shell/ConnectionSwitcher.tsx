// SOT: connection-switcher, quick-connect, sidebar-title
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { engineMeta } from "@/lib/engines";
import { EngineIcon } from "@/components/global/EngineIcon";
import { EnvDot } from "@/components/global/Badge";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";

// WHAT:  Sidebar title that doubles as a quick connection switcher: pick another
//        saved connection (connects on demand), add one, or open the list.
export function ConnectionSwitcher({ caption }: { caption: string }) {
  const connection = useActiveConnection();
  const connections = useWorkspace((s) => s.connections);
  const sessions = useWorkspace((s) => s.sessions);
  const select = useWorkspace((s) => s.selectConnection);
  const goPicker = useWorkspace((s) => s.goPicker);
  const goConnections = useWorkspace((s) => s.goConnections);
  if (!connection) return <span className="text-sm font-medium text-foreground">{caption}</span>;

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
          <EnvDot environment={connection.environment} live />
          <EngineIcon engine={connection.engine} size={16} className="shrink-0" />
          <span className="hidden min-w-0 truncate @[13rem]:inline">{connection.name}</span>
          {/* Part of the trigger, not a chip beside it: the lock describes this
              connection, and at narrow widths a separate badge was an unlabelled
              box sitting next to the engine's brand tile. */}
          {connection.readOnly ? <Icon name="lock" size={11} className="shrink-0 text-muted" /> : null}
          <Icon name="chevron-down" size={12} className="shrink-0 text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="min-w-[320px]">
        <DropdownMenuGroup>
          {connections.map((c) => {
            const meta = engineMeta(c.engine);
            const target = meta.form === "file" ? (c.filePath ?? "") : `${c.host ?? ""}${c.port !== null ? `:${c.port}` : ""}${c.database ? `/${c.database}` : ""}`;
            return (
              <DropdownMenuItem key={c.id} textValue={`${c.name} ${target}`} onSelect={() => { select(c.id); }}>
                <EnvDot environment={c.environment} live={sessions.includes(c.id)} />
                <EngineIcon engine={c.engine} size={20} className="shrink-0" />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate">{c.name}</span>
                  <span className="truncate font-mono text-[10px] text-muted">
                    {meta.label} · {target}
                  </span>
                </span>
                {c.id === connection.id ? <Icon name="check" size={13} className="ml-auto pl-3 text-accent" /> : null}
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuGroup>
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
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
