// SOT: schedules-settings, scheduled-workflows-list, schedules-active-toggle
import { useCallback, useEffect, useState } from "react";
import type { WorkflowScheduleStatus } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";

const REFRESH_MS = 15_000;

const MODE_LABEL = { interval: "Interval", at_time: "At a time" } satisfies Record<WorkflowScheduleStatus["mode"], string>;

function nextLabel(iso: string | null): string {
  if (iso === null) return "—";
  const when = new Date(iso);
  return Number.isNaN(when.getTime()) ? "—" : when.toLocaleString();
}

// WHAT:  Settings → Schedules: every Schedule node in every workflow, with its
//        connection, how it repeats, when it fires next, and whether it is armed.
// WHY:   Schedules run in the background with no window open, so this is the one
//        place to see what the app will do on its own, and to pause it.
// HOW:   The list is read from the core, which knows the real next fire. Pausing or
//        activating saves the workflow with "Schedules active" flipped, exactly like
//        the switch in the workflow's own toolbar.
// WHERE: src-tauri/src/services/workflows/scheduler.rs, src/features/workflows/WorkflowTab.tsx
export function SchedulesSection() {
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const openDocument = useWorkspace((s) => s.openDocument);
  const goWorkspace = useWorkspace((s) => s.goWorkspace);
  const showError = useWorkspace((s) => s.showError);
  const [rows, setRows] = useState<WorkflowScheduleStatus[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const fetchRows = useCallback(() => ipc("list_workflow_schedules"), []);

  useEffect(() => {
    let live = true;
    const refresh = () => {
      void (async () => {
        try {
          const next = await fetchRows();
          if (live) setRows(next);
        } catch (raw) {
          if (!live) return;
          showError(normalizeError(raw));
          setRows((current) => current ?? []);
        }
      })();
    };
    refresh();
    const timer = window.setInterval(refresh, REFRESH_MS);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [fetchRows, showError]);

  const setActive = async (workflowId: string, active: boolean) => {
    setBusy(workflowId);
    try {
      const documents = await ipc("list_documents", { kind: "workflow" });
      const doc = documents.find((d) => d.id === workflowId);
      if (doc?.body.kind !== "workflow") return;
      await saveDocument({ ...doc, body: { kind: "workflow", data: { ...doc.body.data, schedulesActive: active } } });
      setRows(await fetchRows());
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">Schedules</h2>
      <p className="text-xs text-muted">
        Every Schedule node in your workflows. An active workflow runs on its own while the app is open, even with its tab closed. A run is skipped if the previous one is still going.
      </p>
      {rows === null ? (
        <div className="flex items-center gap-2 py-2 text-xs text-muted">
          <Spinner size="sm" />
          Loading schedules…
        </div>
      ) : rows.length === 0 ? (
        <p className="py-2 text-xs text-muted">No schedules yet. Add a Schedule node to a workflow and turn on Schedules active.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((row) => (
            <Card key={`${row.workflowId}:${row.nodeId}`} className="rounded-lg glass-card border-border/40 px-2.5 py-1.5 shadow-xs">
              <CardContent className="flex items-center gap-2 p-0">
                <Icon name="clock" size={14} className="shrink-0 text-muted" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-semibold text-foreground">{row.workflowName}</p>
                  <p className="truncate text-xs text-muted">
                    {row.connectionName ?? "No connection"} · {row.summary}
                  </p>
                  <p className="text-xs text-muted">Next fire: {row.schedulesActive ? nextLabel(row.nextFireAt) : "paused"}</p>
                </div>
                <Badge variant="outline">{MODE_LABEL[row.mode]}</Badge>
                <Badge variant={row.schedulesActive ? "success" : "secondary"}>{row.schedulesActive ? "Active" : "Paused"}</Badge>
                <Button size="sm" variant="secondary" pending={busy === row.workflowId} onClick={() => void setActive(row.workflowId, !row.schedulesActive)}>
                  {row.schedulesActive ? "Pause" : "Activate"}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    goWorkspace();
                    openDocument("workflow", row.workflowId, row.connectionId);
                  }}
                >
                  <Icon name="chevron-right" size={12} />
                  Open
                </Button>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
