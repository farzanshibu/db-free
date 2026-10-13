// SOT: ddl-runner, ddl-preview-debounce, ddl-run-pipeline, schema-form-after-effects
import { useEffect, useState } from "react";
import type { DdlOperation, DdlPreview, TableRef } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { stageStatements, type StageOutcome } from "@/features/changes/stageStatement";

// ============================================================================
// THE PREVIEW -> CONFIRM -> RUN PIPELINE EVERY SCHEMA FORM SHARES
//
// WHAT:  Turns a form's current `DdlOperation` into a previewed script, collects
//        the confirmation the script calls for, runs it, and brings the rest of
//        the app back in line (open tabs, caches, catalog, tags).
// WHY:   Five forms need the same safety story: the SQL is built in Rust and shown
//        before it runs, a destructive script needs a deliberate confirmation, and
//        the read-only lock stays in force. Writing that five times is how one of
//        them would end up skipping a step.
// HOW:   `preview_ddl` is called ~250 ms after the operation settles; a response
//        for an operation the form has since moved past is ignored (it is keyed by
//        the operation's JSON). Running stages each previewed statement in Pending
//        Changes (or commits them in direct mode) via `stageStatements`; the typed
//        name / ticked acknowledgement still gates Run here, and a destructive
//        statement is confirmed again by the commit before it runs.
// WHERE: src-tauri/src/services/schema.rs (preview_ddl), src-tauri/src/guard/mod.rs
// ============================================================================

const PREVIEW_DEBOUNCE_MS = 250;

/// What to bring up to date once the script has run.
export interface DdlEffects {
  /// Toast text on success.
  message: string;
  /// Tables that no longer exist under that name: their tabs close.
  removed: readonly TableRef[];
  /// Tables whose structure or rows changed: an open tab is closed and reopened fresh.
  refresh: readonly TableRef[];
  /// Tag links to follow a rename (`to` set) or forget after a drop (`to` null).
  tagMoves: readonly { schema: string | null; from: string; to: string | null }[];
  /// A table to open afterwards (a new or renamed table).
  open: TableRef | null;
  /// Anything specific to the form (reset the schema filter, …).
  after?: () => void;
}

interface Settled {
  key: string;
  preview: DdlPreview | null;
  error: string | null;
}

export type ConfirmGate = "none" | "name" | "acknowledge";

export interface DdlRunner {
  /// The built script; null while one is being built or the form is incomplete.
  preview: DdlPreview | null;
  previewing: boolean;
  previewError: string | null;
  gate: ConfirmGate;
  confirmText: string;
  setConfirmText: (text: string) => void;
  acknowledged: boolean;
  setAcknowledged: (value: boolean) => void;
  readOnly: boolean;
  canRun: boolean;
  running: boolean;
  runError: string | null;
  run: () => void;
}

// WHAT:  Runs the follow-up work after a script succeeded.
// WHY:   Closing a dropped table's tabs, forgetting cached columns and reloading
//        the catalog is the same list for every form; tag links are cosmetic, so
//        a failure there must never turn a successful DROP into an error toast.
async function applyEffects(connectionId: string, effects: DdlEffects): Promise<void> {
  const workspace = useWorkspace.getState();
  const database = workspace.sessionInfos[connectionId]?.database ?? "";
  for (const move of effects.tagMoves) {
    try {
      await ipc("move_tagged_table", { connectionId, database, schema: move.schema ?? "", from: move.from, to: move.to });
    } catch {
      // Tags are the app's own metadata; the table change already succeeded.
    }
  }
  const reopen = effects.refresh.filter((ref) =>
    workspace.tabs.some((t) => t.kind === "table" && t.connectionId === connectionId && tableKey(t.table) === tableKey(ref)),
  );
  await workspace.schemaChanged(connectionId, [...effects.removed, ...effects.refresh]);
  const after = useWorkspace.getState();
  for (const ref of reopen) after.openTable(connectionId, ref);
  if (effects.open !== null) after.openTable(connectionId, effects.open);
  effects.after?.();
  after.showInfo(effects.message);
}

export function useDdlRunner(connectionId: string, operation: DdlOperation | null, effects: DdlEffects, onDone: () => void): DdlRunner {
  const key = operation === null ? "" : JSON.stringify(operation);
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const [debounced, setDebounced] = useState<{ key: string; operation: DdlOperation | null }>({ key: "", operation: null });
  const [settled, setSettled] = useState<Settled | null>(null);
  // A confirmation belongs to the script it was given for: changing the form
  // (a CASCADE switch, another column dropped) changes the script and asks again.
  const [typed, setTyped] = useState({ script: "", text: "" });
  const [ticked, setTicked] = useState({ script: "", value: false });
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);

  // Wait for the form to go quiet before asking Rust for the statements.
  useEffect(() => {
    const timer = setTimeout(
      () => {
        setDebounced((prev) => (prev.key === key ? prev : { key, operation }));
      },
      key === "" ? 0 : PREVIEW_DEBOUNCE_MS,
    );
    return () => {
      clearTimeout(timer);
    };
  }, [key, operation]);

  useEffect(() => {
    const { key: wanted, operation: op } = debounced;
    if (op === null) return undefined;
    const guard = { stale: false };
    void (async () => {
      try {
        const preview = await ipc("preview_ddl", { connectionId, operation: op });
        if (!guard.stale) setSettled({ key: wanted, preview, error: null });
      } catch (raw) {
        if (!guard.stale) setSettled({ key: wanted, preview: null, error: errorMessage(normalizeError(raw)) });
      }
    })();
    return () => {
      guard.stale = true;
    };
  }, [debounced, connectionId]);

  const current = settled !== null && settled.key === key ? settled : null;
  const preview = current?.preview ?? null;
  const previewError = current?.error ?? null;
  const previewing = key !== "" && current === null;

  const script = preview?.script ?? "";
  const confirmText = typed.script === script ? typed.text : "";
  const acknowledged = ticked.script === script && ticked.value;
  const setConfirmText = (text: string) => {
    setTyped({ script, text });
  };
  const setAcknowledged = (value: boolean) => {
    setTicked({ script, value });
  };
  const confirmName = preview?.confirmName ?? null;
  const gate: ConfirmGate = confirmName !== null ? "name" : preview?.destructive === true ? "acknowledge" : "none";
  const confirmed = gate === "none" || (gate === "name" ? confirmText === confirmName : acknowledged);
  const canRun = preview !== null && !previewing && !readOnly && !running && confirmed;

  // Review mode queues the script's statements in Pending Changes; direct mode commits
  // them now through the same commit path. Either way the follow-up (`applyEffects`)
  // runs once they have landed, and the dialog closes only when the form is done with.
  const run = () => {
    if (preview === null || !canRun) return;
    void (async () => {
      setRunning(true);
      setRunError(null);
      const description = (index: number) => (preview.statements.length > 1 ? `${effects.message} (${index + 1} of ${preview.statements.length})` : effects.message);
      let outcome: StageOutcome;
      try {
        outcome = await stageStatements(
          connectionId,
          preview.statements.map((statement, index) => ({ statement, description: description(index), destructive: preview.destructive })),
          () => applyEffects(connectionId, effects),
        );
      } catch (raw) {
        setRunError(errorMessage(normalizeError(raw)));
        setRunning(false);
        return;
      }
      setRunning(false);
      if (outcome.status === "failed") {
        setRunError(outcome.message);
        return;
      }
      if (outcome.status === "staged") useWorkspace.getState().showInfo(`${preview.statements.length === 1 ? "Statement" : `${preview.statements.length} statements`} added to Pending Changes. Commit them there.`);
      onDone();
    })();
  };

  return { preview, previewing, previewError, gate, confirmText, setConfirmText, acknowledged, setAcknowledged, readOnly, canRun, running, runError, run };
}
