// SOT: command, command-palette-primitives, cmdk
import type * as React from "react";
import { Command as CommandPrimitive } from "cmdk";
import { cn } from "@/lib/cn";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Command (cmdk): a filterable, keyboard-driven list.
// WHY:   ⌘K has to move a roving selection with the arrow keys while focus stays
//        in the input — the one interaction a plain list of buttons cannot do.
// WHERE: https://ui.shadcn.com/docs/components/command
export function Command({ className, ...props }: React.ComponentProps<typeof CommandPrimitive>) {
  return (
    <CommandPrimitive
      data-slot="command"
      className={cn("flex size-full flex-col overflow-hidden rounded-xl bg-transparent text-foreground", className)}
      {...props}
    />
  );
}

export interface CommandDialogProps extends React.ComponentProps<typeof Dialog> {
  title?: string;
  description?: string;
  className?: string;
  children: React.ReactNode;
}

// WHAT:  The palette shell: a dialog that opens near the top of the window with
//        an accessible (but visually hidden) title.
export function CommandDialog({ title = "Command palette", description = "Search commands, tables and saved queries", className, children, ...props }: CommandDialogProps) {
  return (
    <Dialog {...props}>
      <DialogContent showCloseButton={false} className={cn("top-[14vh] max-w-[620px] translate-y-0 overflow-hidden rounded-2xl p-0", className)}>
        <DialogTitle className="sr-only">{title}</DialogTitle>
        <DialogDescription className="sr-only">{description}</DialogDescription>
        {children}
      </DialogContent>
    </Dialog>
  );
}

export function CommandInput({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.Input>) {
  return (
    <div data-slot="command-input-wrapper" className="flex h-11 shrink-0 items-center gap-2 rounded-xl border border-border/40 bg-surface-secondary/40 px-3 glass-input">
      <Icon name="search" aria-hidden className="size-4 shrink-0 text-muted" />
      <CommandPrimitive.Input
        data-slot="command-input"
        className={cn(
          "flex h-full w-full bg-transparent py-3 font-sans text-sm text-foreground outline-none",
          "placeholder:text-field-placeholder disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        {...props}
      />
    </div>
  );
}

export function CommandList({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.List>) {
  return (
    <CommandPrimitive.List
      data-slot="command-list"
      className={cn(
        // Same treatment as ScrollArea: a hairline scrollbar and faded ends, so
        // the palette does not show the platform's default chrome.
        "max-h-[50vh] scroll-py-1 overflow-x-hidden overflow-y-auto p-1",
        "[scrollbar-color:var(--color-surface-tertiary)_transparent] [scrollbar-width:thin]",
        "[mask-image:linear-gradient(to_bottom,transparent_0,black_10px,black_calc(100%-10px),transparent_100%)]",
        className,
      )}
      {...props}
    />
  );
}

export function CommandEmpty({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.Empty>) {
  return <CommandPrimitive.Empty data-slot="command-empty" className={cn("py-6 text-center text-xs text-muted", className)} {...props} />;
}

export function CommandGroup({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.Group>) {
  return (
    <CommandPrimitive.Group
      data-slot="command-group"
      className={cn("overflow-hidden p-0 text-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:text-muted", className)}
      {...props}
    />
  );
}

export function CommandItem({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.Item>) {
  return (
    <CommandPrimitive.Item
      data-slot="command-item"
      className={cn(
        "relative flex cursor-default select-none items-center gap-2.5 rounded-lg px-2.5 py-2 text-xs outline-none",
        "data-[selected=true]:bg-surface-secondary data-[selected=true]:text-foreground",
        "data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-45",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0",
        className,
      )}
      {...props}
    />
  );
}

export function CommandSeparator({ className, ...props }: React.ComponentProps<typeof CommandPrimitive.Separator>) {
  return <CommandPrimitive.Separator data-slot="command-separator" className={cn("-mx-1 h-px bg-separator", className)} {...props} />;
}

export function CommandShortcut({ className, ...props }: React.ComponentProps<"span">) {
  return <span data-slot="command-shortcut" className={cn("ml-auto text-[11px] tracking-widest text-muted", className)} {...props} />;
}
