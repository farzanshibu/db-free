// SOT: redis-key-writes, redis-command-runner, redis-key-existence
import type { QueryOutcome, TableRef } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";

// WHAT:  Running Redis commands on behalf of the key editors, and reading their replies.
// WHY:   Every write is a command line sent through `execute_query`, so the block's
//        read-only lock, destructive confirmation and history apply to it like
//        any statement the user typed. Reads (type, TTL, values) use the session
//        lane commands instead, which a read-only connection may use.
// WHERE: src-tauri/src/guard/mod.rs, src/features/keys/*.tsx

export function keyRef(name: string): TableRef {
  return { schema: null, name };
}

// WHAT:  Runs command lines (one per entry) and returns what the server said.
//        `confirmDestructive` is passed only by a caller that has just asked the
//        user to confirm.
export function runRedis(connectionId: string, lines: readonly string[], confirmDestructive = false): Promise<QueryOutcome> {
  return ipc("execute_query", { connectionId, sql: lines.join("\n"), confirmDestructive, maxRows: 100, schema: null });
}

// WHAT:  The first cell of the `index`-th result of an outcome as an integer.
export function replyInt(outcome: QueryOutcome, index = 0): number | null {
  const statement = outcome.statements[index];
  if (statement?.kind !== "rows") return null;
  const cell = statement.result.rows[0]?.[0];
  if (cell?.t === "int") return cell.v;
  if (cell?.t === "text" || cell?.t === "decimal") {
    const parsed = Number(cell.v);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

export function replyText(outcome: QueryOutcome, index = 0): string | null {
  const statement = outcome.statements[index];
  if (statement?.kind !== "rows") return null;
  const cell = statement.result.rows[0]?.[0];
  if (cell === undefined || cell.t === "null") return null;
  return cell.t === "json" ? JSON.stringify(cell.v) : String(cell.v);
}

// WHAT:  Does a key with this name exist? Answered by the read lane, so it works
//        on a read-only connection and never counts as a write.
export async function keyExists(connectionId: string, name: string): Promise<boolean> {
  try {
    await ipc("key_info", { connectionId, key: name });
    return true;
  } catch (raw) {
    if (normalizeError(raw).kind === "not_found") return false;
    throw raw;
  }
}
