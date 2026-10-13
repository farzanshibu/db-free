// SOT: confirm-popover, destructive-confirmation-popover, key-delete-confirm
import { useState, type ReactElement } from "react";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

// WHAT:  A small "are you sure?" anchored to the button that asks it.
// WHY:   Deleting a key or removing a row is irreversible, and the app's rule is
//        that a destructive action is confirmed by the user before anything is
//        sent with `confirmDestructive`. The button that starts the action is
//        the overlay's trigger, so there is no floating dialog to lose.
// HOW:   Controlled open state so Confirm can close it before the action runs.
// WHERE: src/features/keys/KeyTab.tsx, src/features/keys/CollectionEditor.tsx
interface ConfirmPopoverProps {
  trigger: ReactElement;
  title: string;
  body?: string | undefined;
  confirmLabel: string;
  onConfirm: () => void;
}

export function ConfirmPopover({ trigger, title, body, confirmLabel, onConfirm }: ConfirmPopoverProps) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent align="end" className="w-64 space-y-2 p-2.5">
        <div className="space-y-1">
          <p className="text-xs font-medium text-foreground">{title}</p>
          {body !== undefined ? <p className="text-xs text-muted">{body}</p> : null}
        </div>
        <div className="flex justify-end gap-2">
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setOpen(false);
            }}
          >
            Cancel
          </Button>
          <Button
            size="sm"
            variant="danger"
            onClick={() => {
              setOpen(false);
              onConfirm();
            }}
          >
            {confirmLabel}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
