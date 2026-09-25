// SOT: toaster, toast-region, notifications-ui
import { Toaster as ThemedToaster } from "@/components/ui/sonner";
import { useResolvedTheme } from "@/stores/useTheme";

// WHAT:  The app's toast region, drawn in the resolved app theme.
// WHY:   Sonner paints its own toast background per theme, which outranks the
//        glass-modal class; a hard-coded dark theme left black toasts on the
//        light theme. The primitive stays store-free; this wrapper feeds it.
// WHERE: src/components/ui/sonner.tsx, src/stores/useTheme.ts
export function Toaster() {
  return <ThemedToaster theme={useResolvedTheme()} />;
}
