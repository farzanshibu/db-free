// SOT: app-shell, layout, page-routing, tab-routing, settings-css-vars, tab-shortcuts
import { useEffect } from "react";
import { useActiveConnection, useActiveTab, useWorkspace } from "@/stores/workspace";
import { ipc, normalizeError } from "@/lib/ipc";
import { fontStack } from "@/lib/fonts";
import { IconRail } from "@/features/shell/IconRail";
import { Sidebar } from "@/features/shell/Sidebar";
import { TabBar } from "@/features/shell/TabBar";
import { TabArea } from "@/features/shell/TabView";
import { ConnectionsPage } from "@/features/connections/ConnectionsPage";
import { ConnectionPicker } from "@/features/connections/ConnectionPicker";
import { ConnectionForm } from "@/features/connections/ConnectionForm";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { CapabilityMatrixPage } from "@/features/engines/CapabilityMatrixPage";
import { PendingChangesPanel } from "@/features/changes/PendingChangesPanel";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { BackupDialog } from "@/features/backup/BackupDialog";
import { Toaster } from "@/components/global/Toaster";
import { toast } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useShortcut } from "@/stores/useShortcut";

/// Stacks used until the settings load (they match globals.css).
const UI_FONT_FALLBACK = '"JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
const EDITOR_FONT_FALLBACK = UI_FONT_FALLBACK;

/// Accent colour plus its oklch hue: the hue also drives every tinted surface
/// (window gradient, glass panels, selection) through the --accent-hue variable.
/// Guards the startup update check against running twice. StrictMode invokes
/// every effect a second time in development, and a second run would download
/// the same release again and stack a duplicate toast; a module-level flag
/// survives that remount where component state does not.
let updateChecked = false;

const ACCENTS: Record<string, { color: string; hue: number }> = {
  blue: { color: "oklch(0.6 0.2 258)", hue: 258 },
  violet: { color: "oklch(0.62 0.2 295)", hue: 295 },
  green: { color: "oklch(0.68 0.17 150)", hue: 150 },
  orange: { color: "oklch(0.7 0.17 55)", hue: 55 },
  rose: { color: "oklch(0.64 0.2 10)", hue: 10 },
};

export function App() {
  const bootstrap = useWorkspace((s) => s.bootstrap);
  const ready = useWorkspace((s) => s.ready);
  const page = useWorkspace((s) => s.page);
  const settings = useWorkspace((s) => s.settings);
  const density = useWorkspace((s) => s.density);
  const showError = useWorkspace((s) => s.showError);
  const connection = useActiveConnection();
  const connected = useWorkspace((s) => (connection ? s.sessions.includes(connection.id) : false));
  const tab = useActiveTab();
  const changesOpen = useWorkspace((s) => s.changesPanelOpen);
  const closeTab = useWorkspace((s) => s.closeTab);
  const reopenClosedTab = useWorkspace((s) => s.reopenClosedTab);
  const cycleTab = useWorkspace((s) => s.cycleTab);
  const openQuery = useWorkspace((s) => s.openQuery);

  // WHAT:  Browser-style tab keys: new, close, reopen closed, next / previous.
  useShortcut("new-query", () => {
    const id = tab?.connectionId ?? connection?.id;
    if (id !== undefined) openQuery(id);
  });
  useShortcut("close-tab", () => {
    if (tab) closeTab(tab.id);
  });
  useShortcut("reopen-tab", reopenClosedTab);
  useShortcut("next-tab", () => cycleTab(1));
  useShortcut("prev-tab", () => cycleTab(-1));

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  // WHAT:  Suppress the webview's own right-click menu (Cut/Copy/Paste, Speech,
  //        Inspect Element) app-wide.
  // WHY:   It is a browser menu in a desktop app: it offers nothing the app can
  //        honour and hides the app's own context menus behind a second click.
  // HOW:   Capture phase, preventDefault only — propagation continues, so the
  //        React handlers that open the app's menus still run.
  // WHAT:  One update check at startup. If there is a new version the bytes are
  //        fetched in the background, and the toast that announces it carries
  //        the Restart button that installs them.
  // WHY:   Telling someone to open Settings, find Updates and then wait out a
  //        multi-megabyte download is three steps too many. Downloading first
  //        means the only thing left to ask is when to restart — and restarting
  //        stays their decision, because it closes whatever they are doing.
  // HOW:   `download_update` stages the bytes in the Rust side; `install_update`
  //        applies the staged copy and relaunches. The toast never expires on
  //        its own: it is dismissed by acting on it, or by ignoring it.
  // WHERE: src-tauri/src/commands/updates.rs
  useEffect(() => {
    if (updateChecked) return;
    updateChecked = true;
    void (async () => {
      try {
        const status = await ipc("download_update");
        if (status.available === null) return;
        toast.success(`DB Free ${status.available} is ready to install`, {
          // A fixed id makes this toast a singleton: re-announcing an update
          // replaces the notice instead of stacking another copy of it.
          id: "update-ready",
          description: "The download has finished. Restarting takes a moment.",
          duration: Number.POSITIVE_INFINITY,
          action: {
            label: "Restart",
            onClick: () => {
              void (async () => {
                try {
                  // The app relaunches inside this call, so nothing follows it.
                  await ipc("install_update");
                } catch (raw) {
                  showError(normalizeError(raw));
                }
              })();
            },
          },
        });
      } catch {
        // Offline, or no release feed: an update check is not worth an error toast.
      }
    })();
  }, [showError]);

  // Density drives the chrome's spacing tokens (globals.css), not just row height.
  useEffect(() => {
    document.documentElement.setAttribute("data-density", density);
  }, [density]);

  useEffect(() => {
    const suppress = (event: MouseEvent) => event.preventDefault();
    document.addEventListener("contextmenu", suppress, { capture: true });
    return () => document.removeEventListener("contextmenu", suppress, { capture: true });
  }, []);

  // WHAT:  Settings that are pure CSS (accent, font sizes) apply as root variables.
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement.style;
    const accent = ACCENTS[settings.accent] ?? ACCENTS.blue;
    if (accent) {
      root.setProperty("--accent", accent.color);
      root.setProperty("--accent-hue", String(accent.hue));
    }
    root.setProperty("--font-sans", fontStack(settings.uiFont, UI_FONT_FALLBACK));
    root.setProperty("--font-mono", fontStack(settings.editorFont, EDITOR_FONT_FALLBACK));
    root.setProperty("--ui-font-size", `${settings.uiFontSize}px`);
    root.setProperty("--editor-font-size", `${settings.editorFontSize}px`);
    document.body.style.fontSize = `${settings.uiFontSize}px`;
  }, [settings]);

  return (
    <TooltipProvider>
      <div className="grid-bg flex h-full text-foreground">
      <IconRail />
      {!ready ? null : page.kind === "settings" ? (
        <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
          <SettingsPage />
        </div>
      ) : page.kind === "capabilities" ? (
        <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
          <CapabilityMatrixPage />
        </div>
      ) : page.kind === "connection-picker" ? (
        <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
          <ConnectionPicker />
        </div>
      ) : page.kind === "connection-form" ? (
        <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
          <ConnectionForm />
        </div>
      ) : page.kind === "connections" || !connection || !connected ? (
        <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
          <ConnectionsPage />
        </div>
      ) : (
        <>
          <Sidebar />
          <main className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
            <TabBar />
            <div className="flex min-h-0 flex-1">
              <div className="min-w-0 flex-1">
                <TabArea tab={tab} />
              </div>
              {changesOpen ? <PendingChangesPanel connectionId={connection.id} /> : null}
            </div>
          </main>
        </>
      )}
        <CommandPalette />
        <BackupDialog />
        <Toaster />
      </div>
    </TooltipProvider>
  );
}
