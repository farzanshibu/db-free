// SOT: ddl-dialog-shell, schema-form-frame
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import { DdlPreviewPane } from "./DdlPreviewPane";
import type { DdlRunner } from "./useDdlRunner";

interface DdlDialogProps {
  connectionId: string;
  title: string;
  description: string;
  /// A wide dialog (the column grid) or a compact one (a name and two switches).
  wide?: boolean;
  runner: DdlRunner;
  /// The form is complete: statements can be built from it.
  ready: boolean;
  idleHint: string;
  engineNotes: readonly string[];
  runLabel: string;
  /// Client-side reason Run is off even though a preview exists.
  blocked?: boolean;
  onClose: () => void;
  children: ReactNode;
}

// WHAT:  The frame every schema form sits in: header, scrolling form, the
//        statement preview with its confirmation, and a Cancel / Run footer.
// WHY:   One frame keeps the five forms the same shape, and puts the Run button
//        where its gating (preview built, confirmation given, not read-only) is
//        decided once.
// HOW:   The dialog cannot be dismissed while the script is running: closing
//        mid-flight would leave the user unsure whether the change happened.
// WHERE: src/features/schema/forms/*Dialog.tsx
export function DdlDialog({ connectionId, title, description, wide = false, runner, ready, idleHint, engineNotes, runLabel, blocked = false, onClose, children }: DdlDialogProps) {
  const destructive = runner.preview?.destructive === true;
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !runner.running) onClose();
      }}
    >
      <DialogContent className={cn("max-h-[calc(100dvh-3rem)]", wide ? "h-[min(90vh,860px)] sm:max-w-5xl" : "sm:max-w-lg")}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">{children}</DialogBody>
        <DdlPreviewPane connectionId={connectionId} runner={runner} ready={ready} idleHint={idleHint} engineNotes={engineNotes} tabTitle={title} onOpened={onClose} />
        <DialogFooter>
          <Button size="sm" variant="tertiary" disabled={runner.running} onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" variant={destructive ? "danger" : "primary"} pending={runner.running} disabled={!runner.canRun || blocked} onClick={runner.run}>
            {runLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
