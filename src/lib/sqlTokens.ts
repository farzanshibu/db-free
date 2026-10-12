// SOT: sql-token-registry, raw-sql-token, now-shortcut
import type { SqlToken, Value } from "./bindings";

// WHAT:  The values a cell can be set to that the statement spells itself:
//        the column's DEFAULT and the current date / time.
// WHY:   Typing `now()` into a date cell means "the time of the commit", not the
//        text "now()"; Rust renders the token in the engine's own spelling from a
//        fixed list, so nothing typed here can become arbitrary SQL.
// WHERE: src-tauri/src/services/changes.rs (`token_sql`), src-tauri/src/model/changes.rs (SqlToken)
export const SQL_TOKENS = {
  default: { label: "DEFAULT", shown: "DEFAULT" },
  now: { label: "now()", shown: "now()" },
  current_date: { label: "CURRENT_DATE", shown: "CURRENT_DATE" },
  current_time: { label: "CURRENT_TIME", shown: "CURRENT_TIME" },
  current_timestamp: { label: "CURRENT_TIMESTAMP", shown: "CURRENT_TIMESTAMP" },
} satisfies Record<SqlToken, { label: string; shown: string }>;

/// What the grid shows for a staged cell holding the token (the text it will not be stored as).
export function tokenValue(token: SqlToken): Value {
  return { t: "text", v: SQL_TOKENS[token].shown };
}

const TYPED: ReadonlyMap<string, SqlToken> = new Map([
  ["now()", "now"],
  ["now", "now"],
  ["current_timestamp", "current_timestamp"],
  ["current_timestamp()", "current_timestamp"],
  ["current_date", "current_date"],
  ["current_date()", "current_date"],
  ["current_time", "current_time"],
  ["current_time()", "current_time"],
]);

// WHAT:  Reads `NOW()`, `CURRENT_TIMESTAMP` and the like typed into a date / time cell.
// HOW:   DEFAULT is not recognised from typing: a text column may really hold "DEFAULT";
//        it has its own menu item.
export function tokenFromTyped(text: string): SqlToken | null {
  return TYPED.get(text.trim().toLowerCase()) ?? null;
}
