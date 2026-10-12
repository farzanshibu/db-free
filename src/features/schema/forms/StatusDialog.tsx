// SOT: status-dialog, schema-form-loading-dialog, schema-form-error-dialog
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";

interface StatusDialogProps {
  title: string;
  description: string;
  onClose: () => void;
  /// A failed load: shown as an error, with Retry when `onRetry` is given.
  error?: string | null;
  onRetry?: () => void;
  /// Plain information shown instead of the loading skeleton (an engine's limits).
  notes?: readonly string[];
}

// WHAT:  The small dialog a schema form shows before it has anything to edit:
//        loading, a load that failed, or an explanation that the engine cannot.
// WHY:   Three forms need the same three states; one component keeps them alike.
// WHERE: src/features/schema/forms/ProfileGate.tsx, EditTableDialog.tsx
export function StatusDialog({ title, description, onClose, error = null, onRetry, notes }: StatusDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          {error !== null ? (
            <Alert variant="danger">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle>Could not load</AlertTitle>
                <AlertDescription className="break-words select-text">{error}</AlertDescription>
              </AlertContent>
            </Alert>
          ) : notes !== undefined ? (
            notes.map((note) => (
              <Alert key={note} variant="default">
                <AlertIndicator />
                <AlertContent>
                  <AlertDescription>{note}</AlertDescription>
                </AlertContent>
              </Alert>
            ))
          ) : (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-4 w-3/4 rounded-md" />
              <Skeleton className="h-4 w-1/2 rounded-md" />
            </div>
          )}
        </DialogBody>
        <DialogFooter>
          {error !== null && onRetry !== undefined ? (
            <Button size="sm" variant="secondary" onClick={onRetry}>
              Retry
            </Button>
          ) : null}
          <Button size="sm" variant="tertiary" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
