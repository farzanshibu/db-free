// SOT: ai-access, ai-enabled, data-access-modes, ai-entry-points
import type { AiDataAccess, AppSettings } from "@/lib/bindings";

// WHAT:  What the assistant may do right now, as questions other features can ask.
// WHY:   The editor, the grid and the palette each show an "Ask AI" affordance;
//        they should all agree on when it is available instead of each testing
//        `provider !== "none"` their own way. `provider` already reads `none`
//        when the assistant is switched off (the server derives it), but these
//        helpers say what is meant.
// WHERE: src/features/chat/ChatTab.tsx, src/features/settings/AiSection.tsx

/// A provider is chosen and the assistant is not switched off.
export function aiEnabled(settings: AppSettings | null): boolean {
  return settings !== null && settings.ai.provider !== "none" && settings.ai.dataAccess !== "off";
}

/// The assistant may read rows (its queries run and return data). False for
/// schema-only: callers that would put row values in a prompt must check this.
export function aiCanReadRows(settings: AppSettings | null): boolean {
  return aiEnabled(settings) && settings?.ai.dataAccess === "full";
}

export interface DataAccessOption {
  value: AiDataAccess;
  label: string;
  description: string;
}

export const DATA_ACCESS_OPTIONS: readonly DataAccessOption[] = [
  {
    value: "full",
    label: "Schema & data",
    description: "The assistant can run read-only queries and analyse the rows they return. Those rows are sent to the provider.",
  },
  {
    value: "schema",
    label: "Schema only",
    description: "Table and column names, types and relationships. No row is ever sent; the assistant writes queries for you to run.",
  },
  {
    value: "off",
    label: "Off",
    description: "The assistant is switched off. Nothing about your database is sent anywhere.",
  },
];
