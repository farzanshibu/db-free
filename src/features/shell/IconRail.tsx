// SOT: icon-rail, primary-navigation, window-drag-region
import { useWorkspace, type SidebarMode } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { isMac } from "@/components/global/Kbd";
import { displayChord } from "@/lib/keymap";
import { useKeymap } from "@/stores/useShortcut";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";

const SIDEBARS: readonly { mode: SidebarMode; icon: IconName; label: string }[] = [
  { mode: "tables", icon: "table", label: "Tables" },
  { mode: "objects", icon: "hierarchy", label: "Objects" },
  { mode: "queries", icon: "file", label: "Saved queries" },
  { mode: "dashboards", icon: "columns", label: "Dashboards" },
  { mode: "workflows", icon: "play", label: "Workflows" },
  { mode: "diagrams", icon: "view", label: "Schema diagrams" },
];

// On macOS the traffic lights sit inside this rail, so the two share a centre
// line. wry only moves the buttons horizontally (origin.x = x + i * spacing);
// vertically it re-fits the titlebar container to (button height + y) with the
// top flush to the window, so a button's top offset is `y - 9` — the 9 being
// the buttons' default bottom padding inside the container. Measured on macOS
// 27: buttons are 14x14 with 23px origin spacing, so the cluster spans
// [x, x + 60] and y = (32 - 14) / 2 + 9 = 18 to centre it in the 32px drag
// strip (--titlebar-h), x = (80 - 60) / 2 = 10 to centre it on the 80px rail.
const RAIL_WIDTH_MAC = "w-[80px]";

// WHAT:  Left rail: connections home, one entry per sidebar, history/transfer
//        shortcuts, settings. Its top is the window drag region and, on
//        macOS, it is exactly as wide as the traffic lights.
export function IconRail() {
  const page = useWorkspace((s) => s.page);
  const sidebar = useWorkspace((s) => s.sidebar);
  const activeId = useWorkspace((s) => s.activeConnectionId);
  const connected = useWorkspace((s) => (activeId ? s.sessions.includes(activeId) : false));
  const goConnections = useWorkspace((s) => s.goConnections);
  const goSettings = useWorkspace((s) => s.goSettings);
  const goCapabilities = useWorkspace((s) => s.goCapabilities);
  const setSidebar = useWorkspace((s) => s.setSidebar);
  const openHistory = useWorkspace((s) => s.openHistory);
  const openTransfer = useWorkspace((s) => s.openTransfer);
  const openChat = useWorkspace((s) => s.openChat);
  const openAdmin = useWorkspace((s) => s.openAdmin);
  const setPaletteOpen = useWorkspace((s) => s.setPaletteOpen);
  const collapsed = useWorkspace((s) => s.sidebarCollapsed);
  const toggleSidebar = useWorkspace((s) => s.toggleSidebar);
  const keymap = useKeymap();
  const inWorkspace = page.kind === "workspace";

  return (
    <nav className={cn("flex shrink-0 flex-col items-center glass-dock pb-2", isMac() ? RAIL_WIDTH_MAC : "w-11")} aria-label="Primary">
      <div className={cn("drag-region w-full shrink-0", isMac() ? "app-titlebar" : "h-3")} data-tauri-drag-region />
      <Button
        onClick={() => setPaletteOpen(true)}
        className="group mb-2 flex size-7 min-w-7 items-center justify-center rounded-md bg-accent text-accent-foreground shadow-md shadow-accent/25 liquid-hover hover:scale-105"
        aria-label="Command palette (⌘K)"
      >
        <Icon name="database" size={15} />
      </Button>
      <div className="flex flex-col items-center gap-1">
        <IconButton
          icon="plug"
          label="Connections"
          active={page.kind === "connections" || page.kind === "connection-picker" || page.kind === "connection-form"}
          onClick={goConnections}
          size={15}
        />
        <Separator className="my-1.5 w-5 opacity-40" />
        <IconButton
          icon="sidebar"
          label={`${collapsed ? "Show" : "Hide"} sidebar (${displayChord(keymap["toggle-sidebar"])})`}
          active={inWorkspace && !collapsed}
          disabled={!connected}
          onClick={toggleSidebar}
          size={15}
        />
        {SIDEBARS.map((s) => (
          <IconButton
            key={s.mode}
            icon={s.icon}
            label={s.label}
            active={inWorkspace && !collapsed && sidebar === s.mode}
            disabled={!connected}
            onClick={() => setSidebar(s.mode)}
            size={15}
          />
        ))}
        <Separator className="my-1.5 w-5 opacity-40" />
        <IconButton icon="history" label="Query history" disabled={!connected} onClick={() => activeId && openHistory(activeId)} size={15} />
        <IconButton icon="download" label="Export / Import" disabled={!connected} onClick={() => activeId && openTransfer(activeId)} size={15} />
        <IconButton icon="braces" label="Chat with database" disabled={!connected} onClick={() => activeId && openChat(activeId)} size={15} />
        <IconButton icon="server" label="Server admin" disabled={!connected} onClick={() => activeId && openAdmin(activeId)} size={15} />
      </div>
      <div className="mt-auto flex flex-col items-center gap-1">
        <IconButton icon="grid" label="Engine capabilities" active={page.kind === "capabilities"} onClick={goCapabilities} size={15} />
        <IconButton icon="settings" label="Settings" active={page.kind === "settings"} onClick={goSettings} size={15} />
      </div>
    </nav>
  );
}
