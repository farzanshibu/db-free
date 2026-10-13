// SOT: ddl-preview-pane, ddl-confirmation-gate, ddl-script-preview
import type { ReactNode } from "react";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Check, Field } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import type { DdlRunner } from "./useDdlRunner";
import { ScrollArea } from "@/components/ui/scroll-area";

interface DdlPreviewPaneProps {
  connectionId: string;
  runner: DdlRunner;
  /// The form is complete enough to build statements from.
  ready: boolean;
  /// Shown while it is not: what is still missing.
  idleHint: string;
  /// Caveats that belong to the engine rather than to this change.
  engineNotes: readonly string[];
  /// Title of the query tab "Open in query tab" creates.
  tabTitle: string;
  /// The dialog closes when the script moves to an editor.
  onOpened: () => void;
}

// WHAT:  The statements a form would run, read-only, with what has to be
//        acknowledged before running them.
// WHY:   The user reads the SQL before it runs (the app's rule for anything that
//        changes a schema), can copy it or take it to a query tab to adjust, and
//        meets the confirmation here, next to the thing they are confirming.
// HOW:   Everything shown comes from `DdlRunner`; this component owns no state of
//        its own. A script with a `confirmName` asks for that name typed in full;
//        another destructive one asks for a tick.
// WHERE: src/features/schema/forms/useDdlRunner.ts
export function DdlPreviewPane({ connectionId, runner, ready, idleHint, engineNotes, tabTitle, onOpened }: DdlPreviewPaneProps) {
  const openQuery = useWorkspace((s) => s.openQuery);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const { preview } = runner;

  const copy = async () => {
    if (preview === null) return;
    try {
      await navigator.clipboard.writeText(preview.script);
      showInfo("Statements copied.");
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const body: ReactNode = !ready ? (
    <p className="text-xs text-muted">{idleHint}</p>
  ) : runner.previewError !== null ? (
    <Alert variant="danger">
      <AlertIndicator />
      <AlertContent>
        <AlertTitle>Cannot build the statements</AlertTitle>
        <AlertDescription>{runner.previewError}</AlertDescription>
      </AlertContent>
    </Alert>
  ) : preview === null ? (
    <p className="flex items-center gap-2 text-xs text-muted">
      <Spinner size="sm" /> Building statements…
    </p>
  ) : (
    <>
      <div className="relative overflow-hidden rounded-lg border border-border/60 bg-surface-secondary/60">
        <ScrollArea orientation="both" className="max-h-44">
          <pre className="px-2.5 py-1.5 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground select-text" aria-label="Statements to run">
            {preview.script}
          </pre>
        </ScrollArea>
        <div className="absolute top-1.5 right-1.5 flex gap-1">
          <Button size="xs" variant="secondary" onClick={() => void copy()}>
            <Icon name="copy" size={12} />
            Copy
          </Button>
          <Button
            size="xs"
            variant="secondary"
            onClick={() => {
              openQuery(connectionId, preview.script, tabTitle);
              onOpened();
            }}
          >
            <Icon name="terminal" size={12} />
            Open in query tab
          </Button>
        </div>
      </div>
      {preview.notes.length > 0 ? (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent>
            {preview.notes.map((note) => (
              <AlertDescription key={note}>{note}</AlertDescription>
            ))}
          </AlertContent>
        </Alert>
      ) : null}
      {runner.gate === "name" && preview.confirmName !== null ? (
        <Field
          label={`Type "${preview.confirmName}" to confirm`}
          value={runner.confirmText}
          onChange={runner.setConfirmText}
          placeholder={preview.confirmName}
          mono
          compact
          description="This cannot be undone."
        />
      ) : null}
      {runner.gate === "acknowledge" ? (
        <label className="flex items-center gap-2 text-xs text-foreground">
          <Check checked={runner.acknowledged} onChange={runner.setAcknowledged} label="I understand this cannot be undone" />I understand this cannot be undone
        </label>
      ) : null}
    </>
  );

  return (
    <section aria-label="Preview" className="flex max-h-[46%] shrink-0 flex-col border-t border-border/50 bg-surface/40">
      <ScrollArea className="flex flex-col gap-2 px-3 py-2">
      <div className="flex items-center gap-2 text-[11px] font-semibold tracking-wider text-muted uppercase">
        Preview
        {runner.previewing && ready ? <Spinner size="sm" /> : null}
      </div>
      {runner.readOnly ? (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle>This connection is read-only</AlertTitle>
            <AlertDescription>The statements below can be read and copied, but not run here.</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      {body}
      {engineNotes.map((note) => (
        <p key={note} className="flex items-start gap-1.5 text-[11px] text-muted">
          <Icon name="info" size={12} className="mt-0.5" />
          {note}
        </p>
      ))}
      {runner.runError !== null ? (
        <Alert variant="danger">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle>The statements failed</AlertTitle>
            <AlertDescription className="break-words whitespace-pre-wrap select-text">{runner.runError}</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      </ScrollArea>
    </section>
  );
}
