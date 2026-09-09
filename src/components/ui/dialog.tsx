// SOT: dialog, modal, modal-anatomy
import type * as React from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Dialog (Radix), drawn on the app's glass material.
// WHY:   Every confirm, editor and detail sheet in the app is one of these, so
//        focus trapping, Escape, scroll locking and the backdrop all live here once.
// WHERE: https://ui.shadcn.com/docs/components/dialog
export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogPortal = DialogPrimitive.Portal;
export const DialogClose = DialogPrimitive.Close;

export function DialogOverlay({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Overlay>) {
  return (
    <DialogPrimitive.Overlay
      data-slot="dialog-overlay"
      className={cn(
        "fixed inset-0 z-50 bg-backdrop backdrop-blur-sm",
        "data-[state=open]:animate-in data-[state=open]:fade-in-0",
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0",
        className,
      )}
      {...props}
    />
  );
}

export interface DialogContentProps extends React.ComponentProps<typeof DialogPrimitive.Content> {
  /// Render the corner close button. Off for dialogs that must be answered.
  showCloseButton?: boolean;
}

export function DialogContent({ className, children, showCloseButton = true, ...props }: DialogContentProps) {
  return (
    <DialogPortal>
      <DialogOverlay />
      <DialogPrimitive.Content
        data-slot="dialog-content"
        className={cn(
          "fixed top-1/2 left-1/2 z-50 flex w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 flex-col",
          "max-h-[calc(100dvh-4rem)] rounded-xl glass-modal outline-none",
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          "duration-150",
          className,
        )}
        {...props}
      >
        {children}
        {showCloseButton ? (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            aria-label="Close"
            className="absolute top-3 right-3 grid size-6 place-items-center rounded-md text-muted transition-colors outline-none hover:bg-surface-secondary hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
          >
            <Icon name="x" className="size-3.5" />
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPortal>
  );
}

export function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="dialog-header" className={cn("flex shrink-0 flex-col gap-1 border-b border-border/50 px-4 py-3 pr-10", className)} {...props} />;
}

export function DialogTitle({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return <DialogPrimitive.Title data-slot="dialog-title" className={cn("text-sm font-semibold text-foreground", className)} {...props} />;
}

export function DialogDescription({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return <DialogPrimitive.Description data-slot="dialog-description" className={cn("text-xs text-muted", className)} {...props} />;
}

// WHAT:  The scrolling middle of a dialog: the header and footer stay put.
export function DialogBody({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="dialog-body" className={cn("min-h-0 flex-1 overflow-y-auto px-4 py-3", className)} {...props} />;
}

export function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="dialog-footer" className={cn("flex shrink-0 items-center justify-end gap-2 border-t border-border/50 px-4 py-3", className)} {...props} />;
}
