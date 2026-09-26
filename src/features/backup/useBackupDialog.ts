// SOT: backup-dialog-state, open-backup-dialog
import { create } from "zustand";

// WHAT:  Which connection the Backup / Restore dialog is open for, if any.
// WHY:   Three entry points open it (the connection card's context menu, the
//        sidebar connection menu, ⌘K) and one mounted dialog serves them all;
//        a store of its own keeps that out of the workspace store.
// WHERE: src/features/backup/BackupDialog.tsx (mounted once in App.tsx)
interface BackupDialogState {
  connectionId: string | null;
  open: (connectionId: string) => void;
  close: () => void;
}

export const useBackupDialog = create<BackupDialogState>()((set) => ({
  connectionId: null,
  open: (connectionId) => set({ connectionId }),
  close: () => set({ connectionId: null }),
}));
