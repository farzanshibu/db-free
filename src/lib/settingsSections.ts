// SOT: settings-sections, settings-section-registry, settings-deep-link
import type { IconName } from "@/lib/icons";

// WHAT:  The sections of the Settings page, in nav order.
// WHY:   The page draws its nav from this list and the store's Page carries a
//        section id, so the ⌘K palette can open Settings straight at one
//        ("Keyboard shortcuts") without the store importing a feature.
// WHERE: src/features/settings/SettingsPage.tsx, src/stores/workspace.ts (Page)
export type SettingsSection = "general" | "themes" | "fonts" | "grid" | "editor" | "shortcuts" | "ai" | "security" | "updates" | "advanced";

export const SETTINGS_SECTIONS: readonly { id: SettingsSection; label: string; icon: IconName }[] = [
  { id: "general", label: "General", icon: "settings" },
  { id: "themes", label: "Themes", icon: "eye" },
  { id: "fonts", label: "Fonts", icon: "text" },
  { id: "grid", label: "Data Grid", icon: "table" },
  { id: "editor", label: "Editor", icon: "terminal" },
  { id: "shortcuts", label: "Shortcuts", icon: "hash" },
  { id: "ai", label: "AI", icon: "braces" },
  { id: "security", label: "Security & Privacy", icon: "lock" },
  { id: "updates", label: "Updates", icon: "download" },
  { id: "advanced", label: "Advanced", icon: "columns" },
];
