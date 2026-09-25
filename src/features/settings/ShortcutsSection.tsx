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
  withBinding,
  type ShortcutAction,
} from "@/lib/keymap";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Kbd, KbdGroup } from "@/components/ui/kbd";

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

  return (
    <>
      <div className="flex items-center gap-3">
        <h2 className="text-sm font-semibold text-foreground">Shortcuts</h2>
        <Button size="sm" variant="ghost" className="ml-auto" disabled={bindings.length === 0} onClick={() => onChange([])}>
          <Icon name="refresh" size={12} />
          Reset all
        </Button>
      </div>
      <p className="text-xs text-muted">Click Record, then press the new chord. Esc cancels. Changes apply when you save.</p>
      {conflictCount > 0 ? (
        <Alert variant="warning" className="rounded-xl">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle className="text-xs font-semibold">{conflictCount === 1 ? "A chord is bound twice" : `${conflictCount} chords are bound twice`}</AlertTitle>
            <AlertDescription className="text-xs">Only one of the actions on a shared chord will run. Rebind or reset one of them.</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      {SHORTCUT_GROUPS.map((group) => {
        const actions = SHORTCUT_ACTIONS.filter((a) => SHORTCUTS[a].group === group);
        if (actions.length === 0) return null;
        return (
          <section key={group} className="flex flex-col gap-1.5">
            <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">{group}</h3>
            <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
              {actions.map((action) => {
                const chord = keymap[action];
                const clash = conflicts.get(action);
                const isDefault = chord === normalizeChord(SHORTCUTS[action].keys);
                const live = recording === action;
                return (
                  <li key={action} className="flex items-center gap-3 px-3 py-2 text-[13px]">
                    <div className="min-w-0 flex-1">
                      <p className="text-foreground">{SHORTCUTS[action].label}</p>
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
                    <Button size="icon-sm" variant="ghost" aria-label={`Reset ${SHORTCUTS[action].label}`} title="Reset to default" disabled={isDefault} onClick={() => onChange(withBinding(bindings, action, null))}>
                      <Icon name="refresh" size={12} />
                    </Button>
                  </li>
                );
              })}
            </ul>
          </section>
        );
      })}
      <section className="flex flex-col gap-1.5">
        <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">Fixed</h3>
        <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
          {FIXED.map((s) => (
            <li key={s.keys} className="flex items-center px-3 py-2 text-[13px]">
              <span className="text-foreground">{s.action}</span>
              <span className="ml-auto font-mono text-xs text-muted">{s.keys}</span>
            </li>
          ))}
        </ul>
      </section>
    </>
  );
}
