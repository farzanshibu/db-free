// SOT: transactions-store, manual-transaction-mode, open-transaction-guard
import { create } from "zustand";

// WHAT:  Which connections have a manual transaction open, and which query
//        tabs are in Manual mode.
// WHY:   The transaction belongs to the connection's session (one pinned
//        connection in the adapter), not to a tab: every query tab on that
//        connection runs inside it, and disconnecting or closing the last tab
//        has to know about it before it happens.
// HOW:   Written from the answers of begin/commit/rollback_transaction (each
//        says whether a transaction is open afterwards) and cleared whenever
//        the session is replaced (disconnect, reconnect, database switch),
//        which rolls the adapter's transaction back.
// WHERE: src-tauri/src/integrations/mod.rs (manual transactions), src/features/editor/QueryPane.tsx
interface TransactionsState {
  /// connection id -> a transaction is open on its session.
  open: Record<string, boolean>;
  /// tab id -> the tab is in Manual (explicit COMMIT) mode.
  manual: Record<string, boolean>;
  setOpen: (connectionId: string, open: boolean) => void;
  setManual: (tabId: string, manual: boolean) => void;
  forget: (connectionId: string) => void;
}

export const useTransactions = create<TransactionsState>((set) => ({
  open: {},
  manual: {},
  setOpen: (connectionId, open) => set((s) => ({ open: { ...s.open, [connectionId]: open } })),
  setManual: (tabId, manual) => set((s) => ({ manual: { ...s.manual, [tabId]: manual } })),
  forget: (connectionId) => set((s) => ({ open: { ...s.open, [connectionId]: false } })),
}));

// WHAT:  Asks before an action that ends (or orphans) an open transaction.
// WHY:   Disconnecting rolls the transaction back; closing the last query tab
//        leaves it open with no Commit button in reach. Either loses work
//        silently if nobody asks.
// HOW:   True = go ahead. Nothing open = nothing to ask.
export function confirmLeavingTransaction(connectionId: string, consequence: string): boolean {
  if (useTransactions.getState().open[connectionId] !== true) return true;
  return window.confirm(`A transaction is open on this connection. ${consequence} Continue?`);
}
