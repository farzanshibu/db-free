// SOT: shortcuts-settings, shortcut-recorder, shortcut-conflicts-ui
import { useEffect, useMemo, useState } from "react";
import type { KeyBinding } from "@/lib/bindings";
import {
  SHORTCUTS,
  SHORTCUT_ACTIONS,
  SHORTCUT_GROUPS,
  chordFromEvent,
  chordParts,
  conflictsOf,
  normalizeChord,
  resolveKeymap,
  scopeLabel,
  withBinding,
  type ShortcutAction,
  type ShortcutMeta,
} from "@/lib/keymap";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { SearchInput } from "@/components/ui/input";
import { IconButton } from "@/components/global/Button";
import { WithTip } from "@/components/global/InfoTip";
import { AutoGrid } from "@/components/global/Layout";

/// Keys the app handles but that are not rebindable (native to a widget).
const FIXED: readonly { keys: string; action: string }[] = [
  { keys: "Esc", action: "Close dialogs / cancel edit" },
  { keys: "Enter (in cell)", action: "Stage edit" },
  { keys: "Tab (in editor)", action: "Expand snippet / next field / accept suggestion" },
  { keys: "Middle click (tab)", action: "Close tab" },
];

// WHAT:  Settings → Shortcuts: every registry action grouped by ShortcutGroup,
//        with Record (press the new chord), Reset per row, Reset all, and a
//        warning when two actions share a chord.
// WHY:   Rebinding is only safe when the collision is visible before saving.
// HOW:   Edits the draft's `keybindings` (overrides only; see withBinding). While
//        recording, a capture-phase window listener swallows the key so the
//        chord being recorded does not also fire its current action.
// WHERE: src/lib/keymap.ts (registry), AppSettings.keybindings
export function ShortcutsSection({ bindings, onChange }: { bindings: readonly KeyBinding[]; onChange: (next: KeyBinding[]) => void }) {
  const [recording, setRecording] = useState<ShortcutAction | null>(null);
  const keymap = useMemo(() => resolveKeymap(bindings), [bindings]);
  const conflicts = useMemo(() => conflictsOf(keymap), [keymap]);

  useEffect(() => {
    if (recording === null) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape" && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey) {
        setRecording(null);
        return;
      }
      const chord = chordFromEvent(event);
      if (chord === null) return;
      onChange(withBinding(bindings, recording, chord));
      setRecording(null);
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [bindings, onChange, recording]);

  const conflictCount = new Set([...conflicts.keys()].map((a) => keymap[a])).size;
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const matches = (text: string) => needle.length === 0 || text.toLowerCase().includes(needle);
  const fixed = FIXED.filter((s) => matches(s.action) || matches(s.keys));
  const groups = SHORTCUT_GROUPS.map((group) => ({
    group,
    actions: SHORTCUT_ACTIONS.filter((a) => SHORTCUTS[a].group === group && (matches(SHORTCUTS[a].label) || matches(group))),
  })).filter((g) => g.actions.length > 0);
  const shown = fixed.length + groups.length;

  return (
    <>
      <div className="flex items-center gap-2">
        <h2 className="text-[13px] font-semibold text-foreground">
          <WithTip
            tip="Click Record, then press the new chord. Esc cancels; the cross unbinds. Actions in different areas (SQL editor, data grid) may share a chord. Changes apply when you save."
            tipLabel="About shortcuts"
          >
            Shortcuts
          </WithTip>
        </h2>
        <div className="ml-auto w-56 min-w-0">
          <SearchInput value={query} onChange={setQuery} aria-label="Search shortcuts" placeholder="Search…" className="glass-input w-full rounded-md text-xs" />
        </div>
        <Button size="sm" variant="ghost" disabled={bindings.length === 0} onClick={() => onChange([])}>
          <Icon name="refresh" size={12} />
          Reset all
        </Button>
      </div>
      {conflictCount > 0 ? (
        <Alert variant="warning" className="rounded-lg">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle className="text-xs font-semibold">{conflictCount === 1 ? "A chord is bound twice" : `${conflictCount} chords are bound twice`}</AlertTitle>
            <AlertDescription className="text-xs">Only one of the actions on a shared chord will run. Rebind or reset one of them.</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      {/* WHAT:  Groups flow into as many columns as fit, so the whole keymap
          is one screen on a wide window instead of a long single column. */}
      <AutoGrid min="lg" className="items-start gap-3">
      {groups.map(({ group, actions }) => {
        return (
          <section key={group} className="flex flex-col gap-1.5">
            <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">{group}</h3>
            <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
              {actions.map((action) => {
                const chord = keymap[action];
                const clash = conflicts.get(action);
                const isDefault = chord === normalizeChord(SHORTCUTS[action].keys);
                const live = recording === action;
                const meta: ShortcutMeta = SHORTCUTS[action];
                const scope = scopeLabel(meta.scope);
                return (
                  <li key={action} className="flex items-center gap-2 px-2.5 py-1 text-xs">
                    <div className="min-w-0 flex-1">
                      <p className="flex items-center gap-2 text-foreground">
                        {SHORTCUTS[action].label}
                        {scope !== null ? (
                          <Badge size="sm" variant="soft" className="text-[9.5px] font-medium tracking-wider uppercase">
                            {scope}
                          </Badge>
                        ) : null}
                      </p>
                      {clash !== undefined ? (
                        <p className="mt-0.5 flex items-center gap-1 text-[11px] text-warning">
                          <Icon name="alert" size={11} />
                          Also bound to {clash.map((a) => SHORTCUTS[a].label).join(", ")}
                        </p>
                      ) : null}
                    </div>
                    <span className={cn("min-w-24 text-right", live ? "text-accent" : "text-muted")}>
                      {live ? (
                        <span className="text-xs">Press keys…</span>
                      ) : chord.length === 0 ? (
                        <span className="text-xs">Unbound</span>
                      ) : (
                        <KbdGroup className="justify-end">
                          {chordParts(chord).map((part, i) => (
                            <Kbd key={`${part}-${i}`} className={cn(clash !== undefined && "text-warning")}>
                              {part}
                            </Kbd>
                          ))}
                        </KbdGroup>
                      )}
                    </span>
                    <Button size="sm" variant={live ? "soft" : "secondary"} onClick={() => setRecording(live ? null : action)} aria-pressed={live}>
                      {live ? "Cancel" : "Record"}
                    </Button>
                    <IconButton icon="x" size={12} label={`Unbind ${SHORTCUTS[action].label}`} disabled={chord.length === 0} onClick={() => onChange(withBinding(bindings, action, ""))} />
                    <IconButton icon="refresh" size={12} label={`Reset ${SHORTCUTS[action].label} to default`} disabled={isDefault} onClick={() => onChange(withBinding(bindings, action, null))} />
                  </li>
                );
              })}
            </ul>
          </section>
        );
      })}
      {fixed.length > 0 ? (
      <section className="flex flex-col gap-1.5">
        <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">Fixed</h3>
        <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
          {fixed.map((s) => (
            <li key={s.keys} className="flex items-center px-2.5 py-1 text-xs">
              <span className="text-foreground">{s.action}</span>
              <span className="ml-auto font-mono text-xs text-muted">{s.keys}</span>
            </li>
          ))}
        </ul>
      </section>
      ) : null}
      </AutoGrid>
      {shown === 0 ? <p className="text-xs text-muted">No shortcut matches “{query.trim()}”.</p> : null}
    </>
  );
}
