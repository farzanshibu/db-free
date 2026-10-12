// SOT: query-session-keys, close-session-on-tab-close, schema-cache-refresh-listener
import { ipc, onSchemaCacheUpdated } from "@/lib/ipc";
import { useWorkspace } from "./workspace";

// WHAT:  The key a query tab's runs carry as `ExecuteQueryRequest.sessionKey`.
// WHY:   With a key the Rust core keeps one dedicated database connection per tab
//        (Postgres / MySQL family), so `SET`, temporary tables and an open
//        `BEGIN` are still there on the tab's next Run. The tab id is already
//        unique and lives exactly as long as the tab.
// HOW:   The query-run store passes `sessionKeyFor(tab.id)` on every `execute_query`
//        and batch run. Importing this module is also what installs the cleanup below.
// WHERE: src/stores/queryRuns.ts, src-tauri/src/guard/mod.rs (`statement_in_session`)
export function sessionKeyFor(tabId: string): string {
  return tabId;
}

// WHAT:  Gives the tab's connection back when the tab closes.
// WHY:   A pinned connection otherwise sits open until the idle timeout.
// HOW:   Watches the workspace's tab list rather than hooking `closeTab`, so every
//        way a tab can go away (close, close others, close all, a connection
//        removed) is covered by the one subscription. A failure is ignored: the
//        connection may be gone already, and that closes every session with it.
async function closeQuerySession(connectionId: string, tabId: string): Promise<void> {
  try {
    await ipc("close_session", { connectionId, sessionKey: sessionKeyFor(tabId) });
  } catch {
    // not connected any more: the session went with the connection
  }
}

useWorkspace.subscribe((state, previous) => {
  if (state.tabs === previous.tabs) return;
  const open = new Set(state.tabs.map((tab) => tab.id));
  for (const tab of previous.tabs) {
    if (tab.kind === "query" && !open.has(tab.id)) void closeQuerySession(tab.connectionId, tab.id);
  }
});

// WHAT:  Redraws the sidebar when the background refresh of the persisted schema
//        cache found the live catalog different from the copy it showed at launch.
// WHY:   After a restart `load_catalog` answers from the app store at once and
//        refreshes behind the scenes; this is the moment the refreshed answer
//        reaches the screen. Open tables' column lists are dropped with it, since
//        they may have been served from the same stale copy.
// HOW:   Reloads through the same store action the Refresh button uses.
void onSchemaCacheUpdated((connectionId) => {
  const workspace = useWorkspace.getState();
  if (!workspace.sessions.includes(connectionId)) return;
  const prefix = `${connectionId}:`;
  useWorkspace.setState((s) => ({
    columnsCache: Object.fromEntries(Object.entries(s.columnsCache).filter(([key]) => !key.startsWith(prefix))),
  }));
  void workspace.loadCatalog(connectionId);
});
