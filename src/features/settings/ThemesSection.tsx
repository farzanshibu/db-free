// SOT: themes-settings, theme-gallery, custom-theme-editor, theme-import-export-ui, accent-picker
import { useMemo, useState } from "react";
import type { AppSettings, CustomTheme, ThemeBase } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { pickThemeFile, pickThemeSavePath } from "@/lib/native";
import {
  BUILT_IN_THEMES,
  SYSTEM_THEME,
  THEME_GROUPS,
  THEME_TOKEN_NAMES,
  cssToPickerHex,
  followsAccentSetting,
  newCustomThemeId,
  pickerHexWithAlphaOf,
  resolveActiveTheme,
  snapshotTheme,
  uniqueThemeName,
} from "@/lib/themes";
import { isSafeColor, parseThemeFile, serializeTheme } from "@/lib/themeFile";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { ThemePreview } from "./ThemePreview";

const ACCENTS = [
  { value: "blue", label: "Blue" },
  { value: "violet", label: "Violet" },
  { value: "green", label: "Green" },
  { value: "orange", label: "Orange" },
  { value: "rose", label: "Rose" },
] satisfies readonly { value: string; label: string }[];

type ThemePatch = Partial<Pick<AppSettings, "theme" | "accent" | "customThemes">>;

interface ThemesSectionProps {
  theme: string;
  accent: string;
  customThemes: readonly CustomTheme[];
  /// The operating system's current scheme, so the System card can show what it resolves to.
  system: ThemeBase;
  onChange: (patch: ThemePatch) => void;
}

// WHAT:  Settings → Themes: pick a theme from a gallery of live previews, make
//        your own from any of them, and import or export themes as JSON.
// WHY:   Seven built-in palettes (Dark, Light, Ember, Nocturne, Glacier, Midnight,
//        Parchment) cover most tastes; the editor covers the rest — nine groups of
//        colours (core, surfaces, sidebar, buttons, grid, editor, status, charts,
//        scrollbars) — and a file format lets a theme be shared.
// HOW:   Edits the Settings draft like every other section, so nothing changes in
//        the app until Save. A custom theme starts as a full copy of a built-in
//        palette (`snapshotTheme`), so it never depends on the palette it came
//        from. Imports accept DB Free and DB Pro files; both go through the same
//        validation (no `url()`, only colours, 64 KB at most) before they are kept.
// WHERE: src/lib/themes.ts, src/lib/themeFile.ts, src/features/settings/ThemePreview.tsx
export function ThemesSection({ theme, accent, customThemes, system, onChange }: ThemesSectionProps) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [startFrom, setStartFrom] = useState("dark");
  const [busy, setBusy] = useState(false);

  const active = resolveActiveTheme(theme, system, customThemes);
  const accentApplies = followsAccentSetting(active.id) || (active.custom !== null && !("--accent" in active.custom.colors));
  const editing = customThemes.find((t) => t.id === editingId) ?? null;

  const replace = (next: CustomTheme) => {
    onChange({ customThemes: customThemes.map((t) => (t.id === next.id ? next : t)) });
  };

  const create = (name: string, base: ThemeBase, colors: Readonly<Record<string, string | undefined>>) => {
    const made: CustomTheme = { id: newCustomThemeId(), name: uniqueThemeName(name, customThemes), base, colors: definedColors(colors) };
    onChange({ customThemes: [...customThemes, made], theme: made.id });
    setEditingId(made.id);
  };

  const fromBuiltIn = () => {
    const source = BUILT_IN_THEMES.find((t) => t.id === startFrom) ?? BUILT_IN_THEMES[0];
    if (source === undefined) return;
    create(`My ${source.label}`, source.base, { ...snapshotTheme(source.id) });
  };

  const duplicate = (source: CustomTheme) => {
    create(`${source.name} copy`, source.base, { ...source.colors });
  };

  const remove = (id: string) => {
    onChange({ customThemes: customThemes.filter((t) => t.id !== id), ...(theme === id ? { theme: "dark" } : {}) });
    if (editingId === id) setEditingId(null);
  };

  const importFile = async () => {
    setBusy(true);
    try {
      const path = await pickThemeFile();
      if (path === null) return;
      const parsed = parseThemeFile(await ipc("import_theme_file", { path }), THEME_TOKEN_NAMES);
      if (!parsed.ok) {
        showError(parsed.error);
        return;
      }
      const floor = snapshotTheme(parsed.theme.base);
      create(parsed.theme.name, parsed.theme.base, { ...floor, ...parsed.theme.colors });
      showInfo(
        `Imported “${parsed.theme.name}” from a ${parsed.source === "dbpro" ? "DB Pro" : "DB Free"} theme file${parsed.skipped > 0 ? ` (${String(parsed.skipped)} colours have no equivalent here and were skipped)` : ""}. Save to keep it.`,
      );
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  const exportFile = async (source: CustomTheme) => {
    setBusy(true);
    try {
      const path = await pickThemeSavePath(source.name);
      if (path === null) return;
      const written = await ipc("export_theme_file", { path, contents: serializeTheme({ name: source.name, base: source.base, colors: source.colors }) });
      showInfo(`Saved to ${written}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">Themes</h2>
      <p className="text-xs text-muted">Pick a look, or build your own from any of them. System follows the operating system and switches with it. Changes apply when you save.</p>

      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        <ThemeCard label="System" caption={`Follows the OS (${system})`} selected={theme === SYSTEM_THEME} colors={snapshotTheme(system)} base={system} onSelect={() => { onChange({ theme: SYSTEM_THEME }); }} />
        {BUILT_IN_THEMES.map((t) => (
          <ThemeCard key={t.id} label={t.label} caption={t.base === "dark" ? "Dark" : "Light"} selected={theme === t.id} colors={snapshotTheme(t.id)} base={t.base} onSelect={() => { onChange({ theme: t.id }); }} />
        ))}
        {customThemes.map((t) => (
          <ThemeCard key={t.id} label={t.name} caption={`Custom · ${t.base}`} selected={theme === t.id} colors={t.colors} base={t.base} onSelect={() => { onChange({ theme: t.id }); }} />
        ))}
      </div>

      {accentApplies ? (
        <Card className="rounded-lg glass-card border-border/40 px-2.5 py-1.5 shadow-xs">
          <CardContent className="flex flex-row items-center gap-3 p-0 w-full">
            <div className="min-w-0 flex-1">
              <p className="text-xs font-semibold text-foreground tracking-tight">Accent</p>
              <p className="mt-0.5 text-xs text-muted">Buttons, selections, links.</p>
            </div>
            <AppSelect ariaLabel="Accent" value={accent} options={ACCENTS} onChange={(v) => { onChange({ accent: v }); }} className="w-44" />
          </CardContent>
        </Card>
      ) : (
        <p className="text-xs text-muted">This theme sets its own accent; the Accent setting applies to Dark, Light and custom themes without one.</p>
      )}

      <div className="mt-2 flex items-center gap-2">
        <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">Custom themes</h3>
        <span className="ml-auto flex items-center gap-2">
          <AppSelect ariaLabel="Start from" value={startFrom} options={BUILT_IN_THEMES.map((t) => ({ value: t.id, label: `From ${t.label}` }))} onChange={setStartFrom} className="w-40" size="sm" />
          <Button size="sm" variant="secondary" onClick={fromBuiltIn}>
            <Icon name="plus" size={13} />
            New theme
          </Button>
          <Button size="sm" variant="secondary" pending={busy} onClick={() => void importFile()}>
            <Icon name="download" size={13} />
            Import…
          </Button>
        </span>
      </div>

      {customThemes.length === 0 ? <p className="text-xs text-muted">No custom themes yet. Start one from any theme above, or import a theme file (DB Free or DB Pro).</p> : null}
      <ul className="flex flex-col gap-1.5">
        {customThemes.map((t) => (
          <li key={t.id} className={cn("flex items-center gap-2 rounded-md border bg-surface px-2.5 py-1 text-xs", editingId === t.id ? "border-accent/60" : "border-border")}>
            <span className="min-w-0 flex-1 truncate text-foreground">{t.name}</span>
            <span className="text-[11px] text-muted">{t.base}</span>
            <Button size="sm" variant={editingId === t.id ? "secondary" : "ghost"} onClick={() => { setEditingId(editingId === t.id ? null : t.id); }}>
              <Icon name="pencil" size={12} />
              {editingId === t.id ? "Done" : "Edit"}
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label={`Duplicate ${t.name}`} title="Duplicate" onClick={() => { duplicate(t); }}>
              <Icon name="copy" size={12} />
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label={`Export ${t.name}`} title="Export…" disabled={busy} onClick={() => void exportFile(t)}>
              <Icon name="download" size={12} />
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label={`Delete ${t.name}`} title="Delete" onClick={() => { remove(t.id); }}>
              <Icon name="trash" size={12} />
            </Button>
          </li>
        ))}
      </ul>

      {editing !== null ? <ThemeEditor theme={editing} onChange={replace} /> : null}
    </>
  );
}

function ThemeCard({ label, caption, selected, colors, base, onSelect }: { label: string; caption: string; selected: boolean; colors: Readonly<Record<string, string | undefined>>; base: ThemeBase; onSelect: () => void }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(
        "flex flex-col gap-1.5 rounded-lg border p-2 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
        selected ? "border-accent bg-accent/10" : "border-border bg-surface hover:border-accent/40",
      )}
    >
      <ThemePreview colors={colors} base={base} compact />
      <span className="flex items-center gap-1.5 px-0.5">
        <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">{label}</span>
        {selected ? <Icon name="check" size={13} className="text-accent" /> : null}
      </span>
      <span className="px-0.5 text-[11px] text-muted">{caption}</span>
    </button>
  );
}

// WHAT:  The editor for one custom theme: name, scheme, a live preview, and every
//        colour token in its group.
function ThemeEditor({ theme, onChange }: { theme: CustomTheme; onChange: (next: CustomTheme) => void }) {
  const [openGroup, setOpenGroup] = useState<string | null>("core");
  const colors = useMemo(() => theme.colors, [theme.colors]);

  const setColor = (name: string, value: string) => {
    onChange({ ...theme, colors: { ...theme.colors, [name]: value } });
  };

  return (
    <div className="flex flex-col gap-2 rounded-lg border border-border bg-surface p-2.5">
      <div className="grid grid-cols-[1fr_9rem] gap-2">
        <Field label="Theme name" value={theme.name} onChange={(name) => { onChange({ ...theme, name }); }} />
        <AppSelect<ThemeBase>
          label="Scheme"
          value={theme.base}
          options={[
            { value: "dark", label: "Dark" },
            { value: "light", label: "Light" },
          ]}
          onChange={(base) => { onChange({ ...theme, base }); }}
        />
      </div>
      <p className="text-xs text-muted">The scheme tells the editor, diagrams and the OS window whether the ground is dark or light.</p>
      <ThemePreview colors={colors} base={theme.base} />
      <div className="flex flex-col gap-1">
        {THEME_GROUPS.map((group) => {
          const open = openGroup === group.id;
          return (
            <section key={group.id} className="rounded-lg border border-border/60">
              <button
                type="button"
                aria-expanded={open}
                onClick={() => { setOpenGroup(open ? null : group.id); }}
                className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs font-medium text-foreground outline-none hover:bg-surface-secondary/50 focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                <Icon name={open ? "chevron-down" : "chevron-right"} size={12} className="text-muted" />
                {group.label}
                <span className="ml-auto text-[11px] font-normal text-muted">{group.tokens.length} colours</span>
              </button>
              {open ? (
                <div className="flex flex-col gap-1 border-t border-border/60 px-2.5 py-1.5">
                  <p className="pb-1 text-[11px] text-muted">{group.description}</p>
                  {group.tokens.map((t) => (
                    <TokenRow key={t.name} name={t.name} label={t.label} value={theme.colors[t.name] ?? ""} onChange={(value) => { setColor(t.name, value); }} />
                  ))}
                </div>
              ) : null}
            </section>
          );
        })}
      </div>
    </div>
  );
}

// WHAT:  One colour: a picker and a text box that takes any CSS colour. Only a
//        value that passes the safety check is kept, so the theme never holds
//        something the document would refuse.
function TokenRow({ name, label, value, onChange }: { name: string; label: string; value: string; onChange: (value: string) => void }) {
  const [text, setText] = useState(value);
  const [synced, setSynced] = useState(value);
  // Follow the stored value when it changes from outside (the picker, another row's edit).
  if (value !== synced) {
    setSynced(value);
    setText(value);
  }
  const valid = text.trim().length === 0 || isSafeColor(text);
  return (
    <div className="flex items-center gap-2 text-xs">
      <label className="relative size-6 shrink-0 cursor-pointer overflow-hidden rounded-md border border-border" title="Pick a colour">
        <span className="absolute inset-0" style={{ backgroundColor: valid && text.trim().length > 0 ? text : "transparent" }} />
        <input
          type="color"
          aria-label={`${label} colour`}
          value={cssToPickerHex(valid && text.trim().length > 0 ? text : value)}
          onChange={(event) => {
            const next = pickerHexWithAlphaOf(event.target.value, text.trim().length > 0 && valid ? text : value);
            setText(next);
            setSynced(next);
            onChange(next);
          }}
          className="absolute inset-0 size-full cursor-pointer opacity-0"
        />
      </label>
      <span className="w-44 shrink-0 truncate text-foreground" title={name}>
        {label}
      </span>
      <Input
        value={text}
        aria-label={`${label} value`}
        aria-invalid={!valid}
        spellCheck={false}
        onChange={(event) => {
          const next = event.target.value;
          setText(next);
          if (isSafeColor(next)) {
            setSynced(next.trim());
            onChange(next.trim());
          }
        }}
        className={cn("h-6 flex-1 font-mono text-xs", valid ? "" : "border-danger")}
      />
    </div>
  );
}

// WHAT:  Drops unset tokens so a theme stores only the colours it overrides.
// WHY:   The palette helpers return `undefined` for tokens a base theme leaves
//        alone; the stored theme (and its Rust map) holds strings only.
function definedColors(colors: Readonly<Record<string, string | undefined>>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [token, value] of Object.entries(colors)) {
    if (value !== undefined) out[token] = value;
  }
  return out;
}
