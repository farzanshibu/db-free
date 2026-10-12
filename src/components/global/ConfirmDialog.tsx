// SOT: confirm-dialog, type-the-name-confirmation, destructive-confirm-ui
import { useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  confirmLabel: string;
  /// When set, the confirm button stays disabled until this text is typed exactly:
  /// the "type the name to delete it" gate for actions that cannot be undone.
  requireText?: string;
  /// `danger` for deletes and drops; `primary` for a plain "are you sure".
  tone?: "danger" | "primary";
  /// The confirmed action is running; the button shows progress and cannot be pressed again.
  pending?: boolean;
  onConfirm: () => void | Promise<void>;
  children?: ReactNode;
}

// WHAT:  The shared confirmation dialog: a title, what will happen, and a button
//        that does it, optionally gated behind typing a name.
// WHY:   Deleting a connection, dropping a table or clearing history each used
//        `window.confirm` or nothing. One component means the destructive paths
//        share wording, keyboard handling (Enter confirms once allowed, Escape
//        cancels) and the type-the-name gate.
// HOW:   Controlled. The typed text lives in the inner body, which is keyed by
//        `open`, so reopening starts empty without an effect.
export function ConfirmDialog({ open, onOpenChange, title, description, confirmLabel, requireText, tone = "danger", pending = false, onConfirm, children }: ConfirmDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(next) => { if (!pending) onOpenChange(next); }}>
      <DialogContent className="sm:max-w-[400px]">
        {open ? (
          <ConfirmBody title={title} description={description} confirmLabel={confirmLabel} requireText={requireText} tone={tone} pending={pending} onConfirm={onConfirm} onCancel={() => { onOpenChange(false); }}>
            {children}
          </ConfirmBody>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

interface BodyProps {
  title: string;
  description: ReactNode;
  confirmLabel: string;
  requireText: string | undefined;
  tone: "danger" | "primary";
  pending: boolean;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
  children: ReactNode;
}

function ConfirmBody({ title, description, confirmLabel, requireText, tone, pending, onConfirm, onCancel, children }: BodyProps) {
  const [typed, setTyped] = useState("");
  const allowed = requireText === undefined || typed.trim() === requireText.trim();
  const run = () => {
    if (allowed && !pending) void onConfirm();
  };
  return (
    <>
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        {description !== undefined ? <DialogDescription>{description}</DialogDescription> : null}
      </DialogHeader>
      <DialogBody className="flex flex-col gap-2">
        {children}
        {requireText !== undefined ? (
          <div className="flex flex-col gap-1">
            <Label htmlFor="confirm-name">
              Type <span className="font-mono text-foreground">{requireText}</span> to confirm
            </Label>
            <Input
              id="confirm-name"
              value={typed}
              autoFocus
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => { setTyped(event.target.value); }}
              onKeyDown={(event) => {
                if (event.key === "Enter") run();
              }}
              className="font-mono"
              placeholder={requireText}
            />
          </div>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="tertiary" onClick={onCancel} disabled={pending}>
          Cancel
        </Button>
        <Button variant={tone === "danger" ? "danger" : "primary"} disabled={!allowed} pending={pending} onClick={run}>
          {confirmLabel}
        </Button>
      </DialogFooter>
    </>
  );
}
