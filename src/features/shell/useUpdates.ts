// SOT: update-state, update-check-throttle, update-download-progress, update-install
import { create } from "zustand";
import type { UpdateProgress } from "@/lib/bindings";
import { ipc, normalizeError, onUpdateProgress } from "@/lib/ipc";
import { readStored, writeStored } from "@/lib/storage";
import { toast } from "@/components/ui/sonner";

// WHAT:  Where the app stands with updates: checking, downloading, or holding a
//        downloaded release that only needs a restart.
// WHY:   The startup check, Settings → Updates, the native menu's "Check for
//        Updates" and the update bar all need the same answer; one store means
//        they cannot disagree, and a check started from the menu shows in the bar.
// HOW:   A check asks the release feed and, when something newer exists,
//        downloads it right away (`download_update` stages the bytes in Rust), so
//        the only thing left to ask the user is when to restart. Launch checks
//        are throttled to once every few hours; a manual check always runs.
// WHERE: src-tauri/src/commands/updates.rs, src/features/shell/UpdateBar.tsx

export type UpdatePhase = "idle" | "checking" | "downloading" | "ready" | "error";

interface UpdateState {
  phase: UpdatePhase;
  /// Version of the downloaded (or downloading) release.
  available: string | null;
  notes: string | null;
  progress: UpdateProgress | null;
  error: string | null;
  /// The last completed check found nothing newer than this build.
  upToDate: boolean;
  /// The user closed the bar for this release; it stays closed until another appears.
  dismissed: boolean;
  /// `quiet`: a background check says nothing unless there is an update.
  check: (quiet: boolean) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
}

const CHECKED_AT_KEY = "db-free:update-checked-at";
/// A launch does not look again within this long of the last look (ms).
const LAUNCH_INTERVAL_MS = 6 * 60 * 60 * 1000;

/// Whether a launch-time check is due.
export function launchCheckDue(now: number = Date.now()): boolean {
  const last = Number(readStored(CHECKED_AT_KEY));
  return !Number.isFinite(last) || now - last >= LAUNCH_INTERVAL_MS;
}

function nothing(): void {
  return undefined;
}

export const useUpdates = create<UpdateState>()((set, get) => ({
  phase: "idle",
  available: null,
  notes: null,
  progress: null,
  error: null,
  upToDate: false,
  dismissed: false,

  check: async (quiet) => {
    const phase = get().phase;
    if (phase === "checking" || phase === "downloading") return;
    writeStored(CHECKED_AT_KEY, String(Date.now()));
    set({ phase: "checking", error: null, progress: null, upToDate: false });
    if (!quiet) toast("Checking for updates…", { id: "update-check", duration: 2000 });
    let stop: () => void = nothing;
    try {
      stop = await onUpdateProgress((progress) => {
        set({ phase: "downloading", progress });
      });
    } catch {
      // No event bridge: the check still works, only the progress bar does not.
    }
    try {
      const status = await ipc("download_update");
      if (status.available === null) {
        set({ phase: "idle", available: null, notes: null, progress: null, upToDate: true });
        if (!quiet) toast.success(`DB Free ${status.current} is up to date.`, { id: "update-check" });
        return;
      }
      set({ phase: "ready", available: status.available, notes: status.notes, progress: null, dismissed: false });
      toast.dismiss("update-check");
    } catch (raw) {
      const message = normalizeError(raw).message;
      set({ phase: "error", error: message, progress: null });
      // A background check offline is not worth a toast; one the user asked for is.
      if (!quiet) toast.error(`Could not check for updates: ${message}`, { id: "update-check" });
    } finally {
      stop();
    }
  },

  install: async () => {
    try {
      // The app relaunches inside this call, so nothing follows it.
      await ipc("install_update");
    } catch (raw) {
      set({ phase: "error", error: normalizeError(raw).message });
      toast.error(`Could not install the update: ${normalizeError(raw).message}`);
    }
  },

  dismiss: () => set({ dismissed: true }),
}));
