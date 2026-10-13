// SOT: close-confirm-dialog, unsaved-changes-confirm
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { tabPresentation } from "./TabBar";

// WHAT:  "Close with unsaved changes?" — asked when tabs that registered unsaved
//        work (`setTabDirty`) are about to close.
// WHY:   Closing a designer or a dashboard draft by reflex should not throw the
//        draft away. The question names what would be lost.
// HOW:   Driven by the store's `closeRequest`; "Keep editing" drops the request
//        and nothing closes, "Close" confirms and every requested tab closes
//        (the clean ones with it).
// WHERE: src/stores/workspace.ts (requestCloseTabs, resolveCloseRequest)
export function CloseConfirmDialog() {
  const request = useWorkspace((s) => s.closeRequest);
  const tabs = useWorkspace((s) => s.tabs);
  const resolve = useWorkspace((s) => s.resolveCloseRequest);
  const names = request === null ? [] : tabs.filter((t) => request.dirty.includes(t.id)).map((t) => tabPresentation(t).label);
  const many = names.length > 1;
  return (
    <Dialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) resolve(false);
      }}
    >
      <DialogContent showCloseButton={false} className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Icon name="alert" size={14} className="text-warning" />
            {many ? `Close ${String(names.length)} tabs with unsaved changes?` : "Close this tab with unsaved changes?"}
          </DialogTitle>
          <DialogDescription>
            {names.length > 0 ? `${names.join(", ")} ${many ? "have" : "has"} changes that will be lost.` : "Some changes will be lost."}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button
            size="sm"
            variant="secondary"
            autoFocus
            onClick={() => {
              resolve(false);
            }}
          >
            Keep editing
          </Button>
          <Button
            size="sm"
            variant="danger-soft"
            onClick={() => {
              resolve(true);
            }}
          >
            {many ? "Close tabs" : "Close tab"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
