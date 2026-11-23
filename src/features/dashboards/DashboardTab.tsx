// SOT: dashboard-tab, dashboard-toolbar, dashboard-save, dashboard-fullscreen, dashboard-refresh-interval, dashboard-title, dashboard-side-panel
import { useCallback, useEffect, useMemo, useState } from "react";
import type { DashboardBody, DashboardVariable, Document, Widget } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { DENSITIES } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect } from "@/components/global/Field";
import { EmptyState } from "@/components/global/EmptyState";
import { Shortcut } from "@/components/global/Kbd";
import { SplitLayout } from "@/components/global/Layout";
import { ToolbarActions } from "@/components/global/Toolbar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DashboardGrid } from "./DashboardGrid";
import { DashboardSettings } from "./DashboardSettings";
import { useDashboardResults } from "./dashboardResults";
import { EmptyDashboard } from "./EmptyDashboard";
import { LAYOUT_VERSION, loadLayout, slotForDuplicate, slotForNew, type Cell } from "./layout";
import { TileInspector } from "./TileInspector";
import { useDashboardRuns } from "./useDashboardRuns";
import { useDashboardShortcut } from "./useDashboardShortcut";
import { VariablesToolbar } from "./VariablesToolbar";
import { newWidget, newWidgetId } from "./widgetSql";

// WHAT:  A dashboard: a grid of tiles, each backed by one SQL statement on the
//        dashboard's connection, with variables the statements read, a side
//        panel that edits the selected tile (or the dashboard's own settings),
//        refresh controls, a title, and a fullscreen view.
// WHY:   Dashboards are how a query result stays in front of you. Everything the
//        tab does is about keeping that cheap: tiles re-run only when their
//        statement changes, results survive switching tabs, and nothing is
//        written until Save.
// HOW:   This component owns the editable body and the selection. The pieces:
//        ./DashboardGrid (placement and gestures), ./useDashboardRuns (queries
//        and cache), ./TileInspector (the draft editor), ./DashboardSettings
//        (title and variables), ./VariablesToolbar (changing values).
// WHERE: src-tauri/src/model/documents.rs (DashboardBody), src/features/documents/DocumentTab.tsx

const REFRESH_CHOICES = [
  { value: "0", label: "Manual" },
  { value: "5", label: "Every 5 s" },
  { value: "10", label: "Every 10 s" },
  { value: "30", label: "Every 30 s" },
  { value: "60", label: "Every minute" },
  { value: "300", label: "Every 5 min" },
];

const PANEL_MIN = 300;
const PANEL_MAX = 750;
const PANEL_DEFAULT = 360;
const PANEL_KEY = "db-free:dashboard-panel-width";

/// The body as the editor holds it: positions made valid, in the current grid.
function bodyOf(doc: Document): DashboardBody {
  if (doc.body.kind !== "dashboard") return { widgets: [], variables: [], refreshSeconds: 0, layoutVersion: LAYOUT_VERSION };
  const data = doc.body.data;
  return { ...data, widgets: loadLayout(data.widgets, data.layoutVersion), layoutVersion: LAYOUT_VERSION };
}

/// Whether a key is going into a field (a plain-letter shortcut must leave it alone).
function typingIn(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT");
}

export function DashboardTab({ document: doc, connectionId: initialConnectionId }: { document: Document; connectionId: string | null }) {
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const density = useWorkspace((s) => s.density);
  const connections = useWorkspace((s) => s.connections);
  const activeConnectionId = useWorkspace((s) => s.activeConnectionId);
  const openChat = useWorkspace((s) => s.openChat);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  // An unsaved copy left by this tab earlier (another tab was shown meanwhile) wins
  // over the saved document, unless the document was saved since.
  const [resumed] = useState(() => {
    const copy = useDashboardResults.getState().working[doc.id];
    return copy?.base === doc.updatedAt ? copy : null;
  });
  const [chosenConnectionId, setChosenConnectionId] = useState<string | null>(resumed?.connectionId ?? initialConnectionId ?? activeConnectionId);
  const connectionId = chosenConnectionId ?? activeConnectionId;
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine ?? "postgres");
  const [body, setBody] = useState<DashboardBody>(() => resumed?.body ?? bodyOf(doc));
  const [name, setName] = useState(resumed?.name ?? doc.name);
  const [dirty, setDirty] = useState(resumed !== null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [fresh, setFresh] = useState<ReadonlySet<string>>(new Set());
  const [fullscreen, setFullscreen] = useState(false);
  const setActive = useDashboardResults((s) => s.setActive);
  const clearDrafts = useDashboardResults((s) => s.clearDrafts);
  const setWorking = useDashboardResults((s) => s.setWorking);
  const clearWorking = useDashboardResults((s) => s.clearWorking);
  const setDraft = useDashboardResults((s) => s.setDraft);

  const { runs, running, refresh, preview } = useDashboardRuns({ dashboardId: doc.id, connectionId, engine, widgets: body.widgets, variables: body.variables, refreshSeconds: body.refreshSeconds });

  // Edits to the body, all marking the dashboard unsaved.
  const patchBody = useCallback((partial: Partial<DashboardBody>) => {
    setBody((b) => ({ ...b, ...partial }));
    setDirty(true);
  }, []);
  const setWidgets = useCallback((widgets: Widget[]) => { patchBody({ widgets }); }, [patchBody]);
  // Tile edits are written against the latest list, not the one this render saw.
  const editWidgets = useCallback((change: (widgets: Widget[]) => Widget[]) => {
    setBody((b) => ({ ...b, widgets: change(b.widgets) }));
    setDirty(true);
  }, []);

  // Keep the unsaved copy where the next mount can find it.
  useEffect(() => {
    if (dirty) setWorking(doc.id, { base: doc.updatedAt, name, connectionId, body });
    else clearWorking(doc.id);
  }, [dirty, doc.id, doc.updatedAt, name, connectionId, body, setWorking, clearWorking]);

  const markActive = useCallback(() => { setActive(doc.id); }, [setActive, doc.id]);
  useEffect(() => {
    markActive();
  }, [markActive]);

  // ---------------------------------------------------------- tiles
  const selected = useMemo(() => body.widgets.find((w) => w.id === selectedId) ?? null, [body.widgets, selectedId]);

  const selectTile = (id: string | null) => {
    setSelectedId(id);
    if (id !== null) setSettingsOpen(false);
  };

  const addTile = (cell: Cell | null) => {
    const tile = newWidget({});
    editWidgets((widgets) => {
      const place = cell ?? { ...slotForNew(widgets, tile.w, tile.h), w: tile.w, h: tile.h };
      return [...widgets, { ...tile, x: place.x, y: place.y, w: place.w, h: place.h }];
    });
    setFresh((f) => new Set(f).add(tile.id));
    setSettingsOpen(false);
    setSelectedId(tile.id);
  };

  const removeTile = (id: string) => {
    editWidgets((widgets) => widgets.filter((w) => w.id !== id));
    setDraft(doc.id, id, null);
    setFresh((f) => {
      const next = new Set(f);
      next.delete(id);
      return next;
    });
    if (selectedId === id) setSelectedId(null);
  };

  const duplicateTile = (id: string) => {
    editWidgets((widgets) => {
      const source = widgets.find((w) => w.id === id);
      return source === undefined ? widgets : [...widgets, { ...source, ...slotForDuplicate(widgets, source), id: newWidgetId() }];
    });
  };

  const applyTile = (next: Widget) => {
    editWidgets((widgets) => widgets.map((w) => (w.id === next.id ? next : w)));
    setFresh((f) => {
      const rest = new Set(f);
      rest.delete(next.id);
      return rest;
    });
  };

  // Closing keeps an unsaved edit for next time; Cancel throws it away, and
  // takes a tile that was only just drawn with it.
  const closePanel = () => {
    setSelectedId(null);
    setSettingsOpen(false);
  };

  const cancelTile = () => {
    if (selected === null) return;
    if (fresh.has(selected.id)) removeTile(selected.id);
    else setDraft(doc.id, selected.id, null);
    setSelectedId(null);
  };

  const openSettings = () => {
    setSelectedId(null);
    setSettingsOpen(true);
  };

  // ---------------------------------------------------------- save
  const save = useCallback(async () => {
    const { title, ...rest } = body;
    const trimmed = (title ?? "").trim();
    const data: DashboardBody = { ...rest, layoutVersion: LAYOUT_VERSION, ...(trimmed.length > 0 ? { title: trimmed } : {}) };
    try {
      await saveDocument({ ...doc, name, connectionId, body: { kind: "dashboard", data } });
      clearDrafts(doc.id);
      setDirty(false);
      showInfo("Dashboard saved.");
    } catch (raw) {
      showError(normalizeError(raw));
    }
  }, [body, doc, name, connectionId, saveDocument, clearDrafts, showInfo, showError]);

  // ---------------------------------------------------------- keyboard
  const mine = () => useDashboardResults.getState().activeId === doc.id;
  useDashboardShortcut("dashboard-save", () => {
    if (!mine() || !dirty) return false;
    void save();
    return true;
  });
  useDashboardShortcut("dashboard-refresh", (event) => {
    if (!mine() || typingIn(event.target) || connectionId === null) return false;
    refresh();
    return true;
  });
  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && document.querySelector("[data-radix-popper-content-wrapper]") === null && !typingIn(event.target)) setFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => { window.removeEventListener("keydown", onKey); };
  }, [fullscreen]);

  const askAi = (prompt: string) => {
    if (connectionId === null) return;
    openChat(connectionId, `Build the dashboard "${name}" (document id ${doc.id}). ${prompt}`);
  };

  const anyRunning = running.size > 0;
  const panel = settingsOpen ? "settings" : selected !== null ? "tile" : null;
  const variables = body.variables;
  const setVariables = (next: DashboardVariable[]) => { patchBody({ variables: next }); };

  const pickConnection = (
    <AppSelect
      ariaLabel="Connection"
      value={connectionId ?? ""}
      options={[{ value: "", label: "— connection —" }, ...connections.map((c) => ({ value: c.id, label: c.name }))]}
      onChange={(v) => {
        setChosenConnectionId(v.length > 0 ? v : null);
        setDirty(true);
      }}
      size="sm"
      className="w-32 min-w-0 shrink @2xl:w-44"
      icon="database"
    />
  );

  const surface = (
      <div className="flex h-full min-w-0 flex-1 flex-col">
        <div className={cn("@container flex app-toolbar shrink-0 items-center gap-1 border-b border-border bg-surface whitespace-nowrap", fullscreen ? "pl-24" : "")}>
          <Input
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setDirty(true);
            }}
            className="h-6 w-28 min-w-0 shrink rounded-md bg-transparent text-xs text-foreground @2xl:w-40"
            aria-label="Dashboard name"
          />
          {pickConnection}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button size="sm" variant="ghost" className="shrink-0 text-muted" onClick={refresh} disabled={connectionId === null} aria-label="Refresh">
                <Icon name="refresh" size={13} className={anyRunning ? "animate-spin" : ""} />
                <span className="hidden @3xl:inline">Refresh</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent className="flex items-center gap-1.5">
              Refresh
              <Shortcut action="dashboard-refresh" />
            </TooltipContent>
          </Tooltip>
          <AppSelect ariaLabel="Auto refresh" value={String(body.refreshSeconds)} options={REFRESH_CHOICES} onChange={(v) => { patchBody({ refreshSeconds: Number(v) }); }} size="sm" icon="clock" className="w-28 min-w-0 shrink @2xl:w-32" />
          <VariablesToolbar variables={variables} onChange={setVariables} onEdit={openSettings} />
          <div className="ml-auto flex shrink-0 items-center gap-1 pl-2">
            <ToolbarActions
              foldBelow="xl"
              labelsFrom="3xl"
              menuLabel="More dashboard actions"
              actions={[
                { id: "tile", label: "Add tile", icon: "plus", onSelect: () => { addTile(null); } },
                { id: "settings", label: "Dashboard settings", icon: "settings", active: settingsOpen, priority: "wide", onSelect: openSettings },
                { id: "fullscreen", label: fullscreen ? "Exit fullscreen (Esc)" : "Fullscreen", icon: fullscreen ? "collapse" : "expand", priority: "wide", onSelect: () => { setFullscreen((f) => !f); } },
              ]}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="sm" onClick={() => void save()} disabled={!dirty}>
                  Save{dirty ? " *" : ""}
                </Button>
              </TooltipTrigger>
              <TooltipContent className="flex items-center gap-1.5">
                Save
                <Shortcut action="dashboard-save" />
              </TooltipContent>
            </Tooltip>
          </div>
        </div>
        {connectionId === null ? (
          <EmptyState icon="columns" title="Pick a connection" tip="Every tile runs its query on this connection." action={pickConnection} />
        ) : (
          <>
            {body.title !== undefined && body.title.trim().length > 0 ? <h2 className="shrink-0 px-3 pt-1.5 pb-1 text-center text-[13px] font-semibold text-foreground">{body.title}</h2> : null}
            <DashboardGrid
              widgets={body.widgets}
              runs={runs}
              running={running}
              selectedId={selectedId}
              rowHeight={DENSITIES[density].rowHeight}
              onSelect={selectTile}
              onLayout={setWidgets}
              onCreate={(cell) => { addTile(cell); }}
              onEdit={selectTile}
              onDuplicate={duplicateTile}
              onRemove={removeTile}
              empty={<EmptyDashboard canAsk onAsk={askAi} />}
            />
          </>
        )}
      </div>

  );

  return (
    <div
      data-dashboard={doc.id}
      className={cn("flex h-full min-h-0", fullscreen ? "fixed inset-0 z-40 bg-background" : "")}
      onPointerDownCapture={markActive}
      onFocusCapture={markActive}
    >
      <SplitLayout
        side="right"
        defaultSize={PANEL_DEFAULT}
        min={PANEL_MIN}
        max={PANEL_MAX}
        storageKey={PANEL_KEY}
        collapsed={panel === null}
        resizeLabel="Resize dashboard panel"
        panel={
        <aside className="flex h-full min-h-0 flex-col border-l border-border/40 glass-sidebar select-none" aria-label={panel === "settings" ? "Dashboard settings" : "Tile settings"}>
          {panel === "settings" ? (
            <DashboardSettings
              title={body.title ?? ""}
              onTitle={(title) => { patchBody({ title }); }}
              variables={variables}
              onVariables={setVariables}
              onClose={closePanel}
            />
          ) : selected !== null ? (
            <TileInspector
              key={selected.id}
              dashboardId={doc.id}
              widget={selected}
              isNew={fresh.has(selected.id)}
              connectionId={connectionId}
              engine={engine}
              variables={variables}
              cachedOutcome={runs[selected.id]?.outcome ?? null}
              cachedError={runs[selected.id]?.error ?? null}
              rowHeight={DENSITIES[density].rowHeight}
              preview={preview}
              onApply={applyTile}
              onClose={closePanel}
              onCancel={cancelTile}
              onDelete={() => { removeTile(selected.id); }}
            />
          ) : null}
        </aside>
        }
      >
        {surface}
      </SplitLayout>
    </div>
  );
}
