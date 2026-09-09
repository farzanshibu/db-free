// SOT: kbd-component, platform-modifier
import { Kbd, KbdGroup } from "@/components/ui/kbd";

const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

export function isMac(): boolean {
  return IS_MAC;
}

// WHAT:  The "run this query" chord, spelled for the host platform.
// WHY:   It sits *inside* the Run button rather than beside it, so the action
//        and the way to trigger it read as one control. `className` lets the
//        caller retint the caps for the surface they land on — on the
//        accent-filled Run button the default grey caps would disappear.
export function RunShortcut({ className }: { className?: string }) {
  return (
    <KbdGroup>
      <Kbd className={className}>{IS_MAC ? "⌘" : "Ctrl"}</Kbd>
      <Kbd className={className}>↵</Kbd>
    </KbdGroup>
  );
}
