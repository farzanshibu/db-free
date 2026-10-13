// SOT: settings-page, preferences-ui, ai-settings-ui
import { useState } from "react";
import type { AppSettings, ExecutionMode, RunScope } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { LAYOUT_PRESETS } from "@/lib/layouts";
import { useWorkspace } from "@/stores/workspace";
import { NativeToolPaths } from "@/features/backup/NativeToolPaths";
import { AppSelect, Field, Toggle } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { EDITOR_FONT_OPTIONS, UI_FONT_OPTIONS } from "@/lib/fonts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { SETTINGS_SECTIONS } from "@/lib/settingsSections";
import { AiSection } from "./AiSection";
import { ShortcutsSection } from "./ShortcutsSection";
import { HomeSection } from "./HomeSection";
import { SchedulesSection } from "./SchedulesSection";
import { SnippetsSection } from "./SnippetsSection";
import { SettingsRow as Row } from "./SettingsRow";
import { ThemesSection } from "./ThemesSection";
import { UpdatesSection } from "./UpdatesSection";
import { useSystemTheme } from "@/stores/useTheme";

/// Whole-app zoom stops, as the ⌘+ / ⌘- shortcuts step through them.
const ZOOM_OPTIONS = [
  { value: "0.9", label: "90%" },
  { value: "1", label: "100%" },
  { value: "1.1", label: "110%" },
  { value: "1.25", label: "125%" },
  { value: "1.5", label: "150%" },
] satisfies readonly { value: string; label: string }[];

// WHAT:  Settings page mirroring DB Manager's sections; edits a draft of AppSettings.
//        A floating dock appears while the draft differs from the saved settings
//        (Reset / Save). The AI section keeps its own draft and Save (it carries
//        keys, which are written separately and never echoed): the dock ignores
//        `ai`, and the server keeps the assistant's settings out of this save.
export function SettingsPage() {
  const settings = useWorkspace((s) => s.settings);
  if (!settings) return null;
  return <SettingsBody initial={settings} />;
}

function SettingsBody({ initial }: { initial: AppSettings }) {
  const saveSettings = useWorkspace((s) => s.saveSettings);
  const goConnections = useWorkspace((s) => s.goConnections);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  // The section lives in the store's Page, so ⌘K can open Settings at one.
  const section = useWorkspace((s) => (s.page.kind === "settings" ? (s.page.section ?? "general") : "general"));
  const setSection = useWorkspace((s) => s.goSettingsSection);
  const [draft, setDraft] = useState<AppSettings | null>(initial);
  // Settings saved in another window arrive as a new `initial`. With nothing edited here
  // the draft follows them; with unsaved edits the draft is left alone (Reset picks them up).
  const [seen, setSeen] = useState(initial);
  if (initial !== seen) {
    setSeen(initial);
    if (draft !== null && sameSettings({ ...draft, ai: seen.ai }, seen)) setDraft(initial);
  }
  const systemScheme = useSystemTheme();
  const applyLayout = useWorkspace((s) => s.applyLayout);
  const [saving, setSaving] = useState(false);

  if (!draft) return null;
  // The assistant's settings are the AI section's own (draft, Save and keys); the
  // draft's copy of them is only ever the one the page opened with.
  const dirty = !sameSettings({ ...draft, ai: initial.ai }, initial);
  const reset = () => {
    setDraft(initial);
  };
  const patch = (partial: Partial<AppSettings>) => setDraft((d) => (d ? { ...d, ...partial } : d));

  const save = async () => {
    setSaving(true);
    try {
      await saveSettings(draft, null);
      showInfo("Settings saved.");
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="grid-bg relative flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-2 border-b border-border/40 glass-header" data-tauri-drag-region>
        <Button variant="ghost" size="sm" onClick={goConnections} className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover">
          <Icon name="chevron-left" size={14} />
          Back
        </Button>
        <span className="text-[13px] font-semibold text-foreground tracking-tight" data-tauri-drag-region>
          Preferences
        </span>
        <div className="drag-region h-full flex-1" data-tauri-drag-region />
      </div>
      <div className="flex min-h-0 flex-1">
        <nav className="w-48 shrink-0 space-y-0.5 px-2 py-2.5 glass-sidebar" aria-label="Settings sections">
          {SETTINGS_SECTIONS.map((s) => (
            <Button
              key={s.id}
              variant="ghost"
              size="sm"
              onClick={() => setSection(s.id)}
              className={cn(
                "flex h-6.5 w-full items-center justify-start gap-2 rounded-md px-2 text-left text-xs font-medium liquid-hover",
                section === s.id ? "bg-accent/12 text-accent" : "text-muted hover:bg-surface-secondary/60 hover:text-foreground",
              )}
            >
              <Icon name={s.icon} size={14} />
              {s.label}
            </Button>
          ))}
        </nav>
        <ScrollArea className="min-h-0 flex-1">
          <div className="mx-auto flex w-full max-w-[720px] flex-col gap-3 px-5 py-3">
            {section === "general" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">General</h2>
                <Row title="Confirm destructive statements" body="DROP, TRUNCATE and DELETE/UPDATE without WHERE ask before running.">
                  <Toggle checked={draft.confirmDestructive} onChange={(v) => patch({ confirmDestructive: v })} label="" />
                </Row>
                <Row title="Density" body="Row height and spacing of grids, lists and panels. Compact is the default.">
                  <AppSelect ariaLabel="Density" value={draft.gridDensity} options={[{ value: "compact", label: "Compact" }, { value: "cozy", label: "Cozy" }, { value: "comfortable", label: "Comfortable" }]} onChange={(v) => patch({ gridDensity: v })} className="w-44" />
                </Row>
                <Row title="Open connections in a new window" body="Each connection gets its own window (created, or brought forward if it is already open). New Window is always available from the File menu.">
                  <Toggle checked={draft.openConnectionsInNewWindow} onChange={(v) => patch({ openConnectionsInNewWindow: v })} label="" />
                </Row>
                <Row title="Reuse table preview tab" body="A single click on a table opens an italic preview tab that the next click replaces; double-click, edit or click the tab to keep it. Off opens every table in its own tab.">
                  <Toggle checked={draft.reuseTablePreviewTab} onChange={(v) => patch({ reuseTablePreviewTab: v })} label="" />
                </Row>
                <Row title="Zoom" body="Scales the whole interface. ⌘+ and ⌘- step through the same sizes, ⌘0 resets.">
                  <AppSelect ariaLabel="Zoom" value={String(draft.uiZoom)} options={ZOOM_OPTIONS} onChange={(v) => patch({ uiZoom: Number(v) })} className="w-32" />
                </Row>
                <Row title="Layout" body="Sidebar and inspector arrangements. Applies at once, to this window.">
                  <div className="flex flex-wrap justify-end gap-1.5">
                    {LAYOUT_PRESETS.map((preset) => (
                      <Button key={preset.id} size="sm" variant="secondary" title={preset.description} onClick={() => { applyLayout(preset.id); }}>
                        {preset.label}
                      </Button>
                    ))}
                  </div>
                </Row>
              </>
            ) : null}
            {section === "themes" ? (
              <ThemesSection theme={draft.theme} accent={draft.accent} customThemes={draft.customThemes} system={systemScheme} onChange={patch} />
            ) : null}
            {section === "fonts" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">Fonts</h2>
                <p className="text-xs text-muted">Every family is bundled with the app, so the picker works offline.</p>
                <Row title="Interface font" body="Menus, panels, labels.">
                  <AppSelect ariaLabel="Interface font" value={draft.uiFont} options={UI_FONT_OPTIONS} onChange={(v) => patch({ uiFont: v })} className="w-44" />
                </Row>
                <Row title="Interface font size" body="11–16 px.">
                  <Field label="" type="number" value={String(draft.uiFontSize)} onChange={(v) => patch({ uiFontSize: clamp(Number(v), 11, 16) })} className="w-24 [&_label]:hidden" />
                </Row>
                <Row title="Editor font" body="Query editor, grids and values; monospace only.">
                  <AppSelect ariaLabel="Editor font" value={draft.editorFont} options={EDITOR_FONT_OPTIONS} onChange={(v) => patch({ editorFont: v })} className="w-44" />
                </Row>
                <Row title="Editor font size" body="11–20 px.">
                  <Field label="" type="number" value={String(draft.editorFontSize)} onChange={(v) => patch({ editorFontSize: clamp(Number(v), 11, 20) })} className="w-24 [&_label]:hidden" />
                </Row>
              </>
            ) : null}
            {section === "grid" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">Data Grid</h2>
                <Row title="Alternating row colours" body="Zebra striping in every grid, for reading long rows across.">
                  <Toggle checked={draft.alternatingRows} onChange={(v) => patch({ alternatingRows: v })} label="" />
                </Row>
                <Row title="Remember table sort and filter" body="Reopen each table with the sort and filters you last used, across restarts.">
                  <Toggle checked={draft.rememberTableState} onChange={(v) => patch({ rememberTableState: v })} label="" />
                </Row>
                <Row title="Default page size" body="Rows per page when a table opens. A connection whose page size you change keeps its own.">
                  <AppSelect
                    ariaLabel="Default page size"
                    value={String(draft.defaultPageSize)}
                    options={["25", "50", "100", "200", "500", "1000"].map((n) => ({ value: n, label: `${n} rows` }))}
                    onChange={(v) => patch({ defaultPageSize: Number(v) })}
                    className="w-44"
                  />
                </Row>
                <Row title="Table column preview" body="Expandable column list under each table in the sidebar.">
                  <Toggle checked={draft.columnPreview} onChange={(v) => patch({ columnPreview: v })} label="" />
                </Row>
                <Row title="NULL display" body="Text shown for NULL cells.">
                  <Field label="" value={draft.nullDisplay} onChange={(v) => patch({ nullDisplay: v })} className="w-32 [&_label]:hidden" mono />
                </Row>
                <Row title="Scrollbar size" body="Standard is thin; Large is easier to grab with a mouse.">
                  <AppSelect ariaLabel="Scrollbar size" value={draft.gridScrollbarSize === "large" ? "large" : "standard"} options={[{ value: "standard", label: "Standard" }, { value: "large", label: "Large" }]} onChange={(v) => patch({ gridScrollbarSize: v })} className="w-36" />
                </Row>
                <Row title="Date picker" body="Show a calendar button on date and time fields. Off keeps them text-only, for keyboard editing.">
                  <Toggle checked={draft.datePickerEnabled} onChange={(v) => patch({ datePickerEnabled: v })} label="" />
                </Row>
              </>
            ) : null}
            {section === "editor" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">Query Behaviour</h2>
                <Row title="Max query rows" body="Caps how many rows a query in the editor returns. Larger results are trimmed to this limit to keep the app fast; the results banner says when that happened.">
                  <AppSelect
                    ariaLabel="Max query rows"
                    value={String(draft.maxQueryRows)}
                    options={[
                      { value: "1000", label: "1,000 rows" },
                      { value: "5000", label: "5,000 rows" },
                      { value: "10000", label: "10,000 rows" },
                      { value: "50000", label: "50,000 rows" },
                      { value: "100000", label: "100,000 rows" },
                      { value: "0", label: "Unlimited" },
                    ]}
                    onChange={(v) => patch({ maxQueryRows: Number(v) })}
                    className="w-40"
                  />
                </Row>
                <Row title="Show Results Pane When Running a Query" body="Automatically expand the SQL results pane when you run a query.">
                  <Toggle checked={draft.showResultsPane} onChange={(v) => patch({ showResultsPane: v })} label="" />
                </Row>
                <Row title="Condense SQL When Formatting" body="Keep lists and expressions on a single line when you use Format.">
                  <Toggle checked={draft.condenseSqlWhenFormatting} onChange={(v) => patch({ condenseSqlWhenFormatting: v })} label="" />
                </Row>
                <Row title="Run Without a Selection" body="Run every statement in the editor, or just the statement your cursor is in.">
                  <AppSelect<RunScope> ariaLabel="Run scope" value={draft.runScope} options={[{ value: "all", label: "All statements" }, { value: "current", label: "Current statement" }]} onChange={(v) => patch({ runScope: v })} className="w-48" />
                </Row>
                <Row title="Keep Query Log For" body="Statements the app logs are deleted after this long, when the app starts. The log is what the History tab and the editor's history panel show.">
                  <AppSelect
                    ariaLabel="Query log retention"
                    value={String(draft.logRetentionDays)}
                    options={[
                      { value: "7", label: "7 days" },
                      { value: "14", label: "14 days" },
                      { value: "30", label: "30 days" },
                      { value: "60", label: "60 days" },
                      { value: "90", label: "90 days" },
                      { value: "0", label: "Forever" },
                    ]}
                    onChange={(v) => patch({ logRetentionDays: Number(v) })}
                    className="w-40"
                  />
                </Row>
                <Row title="Keep AI Prompt as a Comment" body="When the editor's inline AI (⌘I) writes code, put your instruction above it as a comment.">
                  <Toggle checked={draft.inlineAiPromptAsComment} onChange={(v) => patch({ inlineAiPromptAsComment: v })} label="" />
                </Row>
                <Row title="Execution Mode" body="Review Mode queues changes for review before applying. Direct Mode applies changes immediately.">
                  <AppSelect<ExecutionMode> ariaLabel="Execution mode" value={draft.executionMode} options={[{ value: "review", label: "Review Mode" }, { value: "direct", label: "Direct Mode" }]} onChange={(v) => patch({ executionMode: v })} className="w-48" />
                </Row>
                <Separator />
                <h2 className="text-[13px] font-semibold text-foreground">Command Menu</h2>
                <p className="text-xs text-muted">Order of sections in the command menu (⌘K). Use the arrows to reorder and the eye to hide a section.</p>
                <ul className="flex flex-col gap-1">
                  {draft.commandMenuSections.map((name, i) => {
                    const hidden = draft.commandMenuHiddenSections.includes(name);
                    return (
                    <li key={name} className="flex items-center gap-2 rounded-md border border-border bg-surface px-2.5 py-1 text-xs">
                      <span className={cn("capitalize", hidden && "text-muted line-through")}>{name.replace("_", " ")}</span>
                      <span className="ml-auto flex gap-1">
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={hidden ? `Show ${name.replace("_", " ")}` : `Hide ${name.replace("_", " ")}`}
                          aria-pressed={hidden}
                          onClick={() => patch({ commandMenuHiddenSections: hidden ? draft.commandMenuHiddenSections.filter((h) => h !== name) : [...draft.commandMenuHiddenSections, name] })}
                        >
                          <Icon name={hidden ? "eye-off" : "eye"} size={12} />
                        </Button>
                        <Button size="sm" variant="ghost" aria-label="Move up" disabled={i === 0} onClick={() => patch({ commandMenuSections: move(draft.commandMenuSections, i, i - 1) })}><Icon name="arrow-up" size={12} /></Button>
                        <Button size="sm" variant="ghost" aria-label="Move down" disabled={i === draft.commandMenuSections.length - 1} onClick={() => patch({ commandMenuSections: move(draft.commandMenuSections, i, i + 1) })}><Icon name="arrow-down" size={12} /></Button>
                      </span>
                    </li>
                    );
                  })}
                </ul>
                <h2 className="text-[13px] font-semibold text-foreground">Inspector Tabs</h2>
                <p className="text-xs text-muted">Order of the tabs in the record inspector. The top tab is the default.</p>
                <ul className="flex flex-col gap-1">
                  {draft.inspectorTabs.map((name, i) => (
                    <li key={name} className="flex items-center gap-2 rounded-md border border-border bg-surface px-2.5 py-1 text-xs">
                      <span className="uppercase">{name}</span>
                      <span className="ml-auto flex gap-1">
                        <Button size="sm" variant="ghost" aria-label="Move up" disabled={i === 0} onClick={() => patch({ inspectorTabs: move(draft.inspectorTabs, i, i - 1) })}><Icon name="arrow-up" size={12} /></Button>
                        <Button size="sm" variant="ghost" aria-label="Move down" disabled={i === draft.inspectorTabs.length - 1} onClick={() => patch({ inspectorTabs: move(draft.inspectorTabs, i, i + 1) })}><Icon name="arrow-down" size={12} /></Button>
                      </span>
                    </li>
                  ))}
                </ul>
                <Separator />
                <SnippetsSection snippets={draft.snippets} onChange={(snippets) => patch({ snippets })} />
              </>
            ) : null}
            {section === "shortcuts" ? <ShortcutsSection bindings={draft.keybindings} onChange={(keybindings) => patch({ keybindings })} /> : null}
            {section === "ai" ? <AiSection /> : null}
            {section === "security" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">Security & Privacy</h2>
                <Row title="Telemetry" body="Zero telemetry, always. Nothing leaves this machine except your own database and AI requests.">
                  <span className="text-xs text-success">off</span>
                </Row>
                <Row title="Crash reports" body="Opt-in only. Off by default; this build keeps them local.">
                  <Toggle checked={draft.crashReportsOptIn} onChange={(v) => patch({ crashReportsOptIn: v })} label="" />
                </Row>
                <Row title="Credentials" body="Passwords and API keys are sealed with AES-256-GCM; the master key lives in the OS keychain.">
                  <Icon name="lock" size={14} className="text-muted" />
                </Row>
              </>
            ) : null}
            {section === "updates" ? <UpdatesSection checkOnLaunch={draft.checkUpdatesOnLaunch} onCheckOnLaunch={(v) => patch({ checkUpdatesOnLaunch: v })} /> : null}
            {section === "schedules" ? <SchedulesSection /> : null}
            {section === "home" ? (
              <HomeSection showPinned={draft.showPinned} showRecents={draft.showRecents} onChange={(partial) => patch(partial)} />
            ) : null}
            {section === "advanced" ? (
              <>
                <h2 className="text-[13px] font-semibold text-foreground">Advanced</h2>
                <Card className="rounded-lg glass-card border-border/40 px-3 py-2 shadow-xs">
                  <CardContent className="flex flex-col gap-2 p-0">
                    <div>
                      <p className="text-xs font-semibold text-foreground tracking-tight">Backup tools</p>
                      <p className="text-xs text-muted mt-0.5">Backup / Restore runs pg_dump, mysqldump and mongodump. Leave a field empty to use PATH, or point it at the program or its folder.</p>
                    </div>
                    <NativeToolPaths value={draft.nativeToolPaths} onChange={(nativeToolPaths) => patch({ nativeToolPaths })} />
                  </CardContent>
                </Card>
                <Row title="Reset preferences" body="Restores every setting to its default (connections and saved queries are kept).">
                  <Button size="sm" variant="danger-soft" onClick={() => setDraft((d) => (d ? { ...defaultSettings(), ai: d.ai } : d))}>
                    Reset
                  </Button>
                </Row>
              </>
            ) : null}
          </div>
        </ScrollArea>
      </div>
      {dirty ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-5 flex justify-center">
          <div role="status" className="pointer-events-auto flex items-center gap-2 rounded-lg glass-modal py-1 pr-1.5 pl-2.5 shadow-xl">
            <Icon name="info" size={14} className="text-accent" />
            <span className="mr-3 text-xs font-medium text-foreground">Unsaved changes</span>
            <Button size="sm" variant="danger-soft" onClick={reset} disabled={saving} className="rounded-lg liquid-hover">
              Reset
            </Button>
            <Button size="sm" pending={saving} onClick={() => void save()} className="font-semibold liquid-hover">
              Save
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

// WHAT:  Structural equality for the draft vs saved settings; a fixed key list
//        keeps JSON.stringify order-independent (top level, `ai`, and the
//        objects inside the list fields — the replacer list filters every
//        level, so a nested key missing from it would compare as equal).
function sameSettings(a: AppSettings, b: AppSettings): boolean {
  const nested = [...a.keybindings, ...b.keybindings, ...a.snippets, ...b.snippets].flatMap((k) => Object.keys(k));
  // A custom theme is an object of colours keyed by token name: every one of those names counts.
  const themeKeys = [...a.customThemes, ...b.customThemes].flatMap((t) => [...Object.keys(t), ...Object.keys(t.colors)]);
  const keys = [...new Set([...Object.keys(a), ...Object.keys(a.ai), ...Object.keys(a.nativeToolPaths), ...Object.keys(b.nativeToolPaths), ...nested, ...themeKeys])].sort();
  return JSON.stringify(a, keys) === JSON.stringify(b, keys);
}

function clamp(n: number, lo: number, hi: number): number {
  return Number.isFinite(n) ? Math.min(hi, Math.max(lo, Math.round(n))) : lo;
}

function move<T>(list: readonly T[], from: number, to: number): T[] {
  const copy = [...list];
  const [item] = copy.splice(from, 1);
  if (item !== undefined) copy.splice(to, 0, item);
  return copy;
}

function defaultSettings(): AppSettings {
  return {
    theme: "dark",
    accent: "blue",
    uiFont: "jetbrains-mono",
    editorFont: "jetbrains-mono",
    uiFontSize: 12,
    editorFontSize: 13,
    gridDensity: "compact",
    alternatingRows: true,
    rememberTableState: true,
    columnPreview: true,
    maxQueryRows: 10_000,
    nullDisplay: "NULL",
    showResultsPane: true,
    condenseSqlWhenFormatting: false,
    runScope: "all",
    executionMode: "review",
    commandMenuSections: ["create", "navigation", "connections", "tables", "saved_queries", "dashboards", "workflows", "diagrams", "settings"],
    inspectorTabs: ["fields", "json", "sql"],
    confirmDestructive: true,
    crashReportsOptIn: false,
    ai: { provider: "none", model: "claude-opus-5", baseUrl: null, hasApiKey: false, autonomy: "ask_on_write", dataAccess: "full", defaultProfile: null, profiles: [], maxSteps: 12 },
    nativeToolPaths: {},
    keybindings: [],
    snippets: [],
    openConnectionsInNewWindow: false,
    uiZoom: 1,
    reuseTablePreviewTab: false,
    gridScrollbarSize: "standard",
    datePickerEnabled: true,
    commandMenuHiddenSections: [],
    customThemes: [],
    checkUpdatesOnLaunch: true,
    logRetentionDays: 30,
    inlineAiPromptAsComment: true,
    showPinned: true,
    showRecents: true,
    defaultPageSize: 50,
  };
}
