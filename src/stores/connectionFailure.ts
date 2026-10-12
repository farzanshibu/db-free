// SOT: connection-failure-state, connection-failed-dialog-state
import { create } from "zustand";

export interface ConnectionFailure {
  connectionId: string;
  name: string;
  message: string;
}

interface ConnectionFailureState {
  failure: ConnectionFailure | null;
  report: (failure: ConnectionFailure) => void;
  dismiss: () => void;
}

// WHAT:  The last connect attempt that failed, until the user dismisses it.
// WHY:   A toast disappears while the message is still being read, and says
//        nothing about what to check. The connections page shows this in a
//        dialog with a checklist and a way straight back to the connection's
//        settings; the workspace store reports into it from `connect`.
// WHERE: src/features/connections/ConnectionFailedDialog.tsx, src/stores/workspace.ts (connect)
export const useConnectionFailure = create<ConnectionFailureState>((set) => ({
  failure: null,
  report: (failure) => { set({ failure }); },
  dismiss: () => { set({ failure: null }); },
}));
