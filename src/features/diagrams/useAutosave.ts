// SOT: use-autosave, document-autosave, debounced-save, automatic-versions-cadence
import { useCallback, useEffect, useRef, useState } from "react";
import type { AppError, Document } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";

// WHAT:  Saves a document a moment after the last edit, and files an automatic
//        version of it a little later still.
// WHY:   The designer has no Save button to forget. A save is cheap, so it follows
//        the edit closely; a version is a place to come back to, so it waits for a
//        pause — filing one per drag would push every useful state out of the twenty
//        the history keeps.
// HOW:   `touch()` after every edit bumps a generation counter and restarts two
//        timers (save: short, version: longer). Saves run one at a time. A save
//        finishing marks only the generation it started from as saved, so an edit
//        made mid-save is still waiting. Closing the view flushes both.
// WHERE: src-tauri/src/services/document_versions.rs (autosave), src/features/diagrams/DesignerTab.tsx

export type SaveState = "saved" | "dirty" | "saving" | "error";

export interface Autosave {
  state: SaveState;
  /// When the last save finished.
  savedAt: Date | null;
  /// Call after every edit.
  touch: () => void;
  /// Writes now whatever is waiting. With `version`, also files an automatic version.
  flush: (options?: { version?: boolean }) => Promise<void>;
}

const SAVE_DELAY_MS = 800;
const VERSION_DELAY_MS = 8000;

interface Params {
  /// The document as it should be saved right now.
  build: () => Document;
  /// A save finished; `saved` is what the store now holds.
  onSaved: (saved: Document) => void;
  onError: (error: AppError) => void;
}

export function useAutosave({ build, onSaved, onError }: Params): Autosave {
  const [state, setState] = useState<SaveState>("saved");
  const [savedAt, setSavedAt] = useState<Date | null>(null);
  const generation = useRef(0);
  const savedGeneration = useRef(0);
  const versionedGeneration = useRef(0);
  const saveTimer = useRef<number | null>(null);
  const versionTimer = useRef<number | null>(null);
  const queue = useRef<Promise<void>>(Promise.resolve());
  /// True once the view is closing; its last save has nobody left to tell about a vanished document.
  const closing = useRef(false);
  const latest = useRef({ build, onSaved, onError });
  useEffect(() => {
    latest.current = { build, onSaved, onError };
  });

  const run = useCallback(async (wantVersion: boolean) => {
    const needSave = generation.current > savedGeneration.current;
    const needVersion = wantVersion && generation.current > versionedGeneration.current;
    if (!needSave && !needVersion) return;
    const started = generation.current;
    setState("saving");
    try {
      const saved = await ipc("autosave_document", { document: latest.current.build(), takeVersion: needVersion });
      savedGeneration.current = Math.max(savedGeneration.current, started);
      if (needVersion) versionedGeneration.current = Math.max(versionedGeneration.current, started);
      setSavedAt(new Date());
      setState(generation.current > savedGeneration.current ? "dirty" : "saved");
      latest.current.onSaved(saved);
    } catch (raw) {
      setState("error");
      const error = normalizeError(raw);
      // A document deleted while its tab was open has nothing left to save to; saying
      // so as the tab goes away would only be noise.
      if (closing.current && error.kind === "not_found") return;
      latest.current.onError(error);
    }
  }, []);

  const flush = useCallback(
    (options?: { version?: boolean }) => {
      for (const timer of [saveTimer, versionTimer]) {
        if (timer.current !== null) window.clearTimeout(timer.current);
        timer.current = null;
      }
      const next = queue.current.then(() => run(options?.version === true));
      queue.current = next;
      return next;
    },
    [run],
  );

  const touch = useCallback(() => {
    generation.current += 1;
    setState((s) => (s === "saving" ? s : "dirty"));
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      saveTimer.current = null;
      queue.current = queue.current.then(() => run(false));
    }, SAVE_DELAY_MS);
    if (versionTimer.current !== null) window.clearTimeout(versionTimer.current);
    versionTimer.current = window.setTimeout(() => {
      versionTimer.current = null;
      queue.current = queue.current.then(() => run(true));
    }, VERSION_DELAY_MS);
  }, [run]);

  // Closing the view saves and versions whatever is still waiting.
  useEffect(() => {
    closing.current = false;
    return () => {
      closing.current = true;
      void flush({ version: true });
    };
  }, [flush]);

  return { state, savedAt, touch, flush };
}
