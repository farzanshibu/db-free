// SOT: connection-failed-dialog, connect-failure-checklist-ui
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { useConnectionFailure } from "@/stores/connectionFailure";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { connectionFailureHints } from "./failureHints";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  "Connection failed": what happened, a checklist of what to check, and
//        the way back to the connection's settings.
// WHY:   A toast named the error and vanished. This stays until read, offers
//        Retry and Edit, and tailors the checklist to the failure.
// HOW:   Mounted by the connections page; the workspace store's `connect`
//        reports into `useConnectionFailure` when the page is showing.
export function ConnectionFailedDialog() {
  const failure = useConnectionFailure((s) => s.failure);
  const dismiss = useConnectionFailure((s) => s.dismiss);
  const connection = useWorkspace((s) => s.connections.find((c) => c.id === failure?.connectionId) ?? null);
  const connecting = useWorkspace((s) => s.connecting !== null);
  const connect = useWorkspace((s) => s.connect);
  const openForm = useWorkspace((s) => s.openForm);

  return (
    <Dialog open={failure !== null} onOpenChange={(open) => { if (!open) dismiss(); }}>
      <DialogContent className="sm:max-w-[500px]">
        {failure ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-1.5">
                <Icon name="alert" size={14} className="text-danger" />
                Connection failed
              </DialogTitle>
              <DialogDescription>Unable to connect to “{failure.name}”.</DialogDescription>
            </DialogHeader>
            <DialogBody className="flex flex-col gap-2">
              <p className="text-xs text-muted">Please check your connection settings and ensure:</p>
              <ul className="ml-4 list-disc space-y-1 text-xs text-muted">
                {connectionFailureHints(failure.message, connection).map((hint) => (
                  <li key={hint}>{hint}</li>
                ))}
              </ul>
              <div className="overflow-hidden rounded-lg border border-danger/30 bg-danger/10">
                <ScrollArea orientation="both" className="max-h-40">
                  <pre className="selectable p-2 font-mono text-[11px] break-words whitespace-pre-wrap text-foreground">{failure.message}</pre>
                </ScrollArea>
              </div>
            </DialogBody>
            <DialogFooter>
              <Button variant="tertiary" onClick={dismiss}>
                Close
              </Button>
              <Button
                variant="secondary"
                onClick={() => {
                  dismiss();
                  openForm(failure.connectionId);
                }}
              >
                <Icon name="pencil" size={13} />
                Edit connection
              </Button>
              <Button
                pending={connecting}
                onClick={() => {
                  const id = failure.connectionId;
                  dismiss();
                  void connect(id);
                }}
              >
                <Icon name="refresh" size={13} />
                Retry
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
