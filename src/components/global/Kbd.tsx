// SOT: kbd-component, platform-modifier, shortcut-keycaps
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { IS_MAC, chordParts, type ShortcutAction } from "@/lib/keymap";
import { useKeymap } from "@/stores/useShortcut";

export function isMac(): boolean {
  return IS_MAC;
}

// WHAT:  An action's current chord as keycaps, following the user's rebinds.
// WHY:   A hint that still says ⌘S after the user moved Commit to ⌘⇧S teaches
//        the wrong key. Reading the live keymap keeps every hint honest.
// HOW:   Enter is drawn as ↵ so a chord fits inside a compact button.
// WHERE: src/lib/keymap.ts (registry, chordParts), AppSettings.keybindings
export function Shortcut({ action, className }: { action: ShortcutAction; className?: string | undefined }) {
  const keymap = useKeymap();
  const parts = chordParts(keymap[action]);
  if (parts.length === 0) return null;
  return (
    <KbdGroup>
      {parts.map((part, i) => (
        <Kbd key={`${part}-${i}`} className={className}>
          {part === "Enter" || part === "↩" ? "↵" : part}
        </Kbd>
      ))}
    </KbdGroup>
  );
}

// WHAT:  The "run this query" chord, spelled for the host platform.
// WHY:   It sits *inside* the Run button rather than beside it, so the action
//        and the way to trigger it read as one control. `className` lets the
//        caller retint the caps for the surface they land on — on the
//        accent-filled Run button the default grey caps would disappear.
export function RunShortcut({ className }: { className?: string }) {
  return <Shortcut action="run" className={className} />;
}
