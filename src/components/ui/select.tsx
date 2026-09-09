// SOT: select, listbox, option-picker
import type * as React from "react";
import { Select as SelectPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Select (Radix).
// WHY:   Radix Select speaks plain strings, unlike the collection APIs it
//        replaces — which is what lets `AppSelect` stay generic over the
//        caller's own string union with no casting.
// WHERE: https://ui.shadcn.com/docs/components/select
export const Select = SelectPrimitive.Root;
export const SelectGroup = SelectPrimitive.Group;
export const SelectValue = SelectPrimitive.Value;

export interface SelectTriggerProps extends React.ComponentProps<typeof SelectPrimitive.Trigger> {
  size?: "sm" | "md";
}

export function SelectTrigger({ className, size = "md", children, ...props }: SelectTriggerProps) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      className={cn(
        "flex w-full min-w-0 items-center justify-between gap-1.5 rounded-[var(--field-radius)] border border-input bg-field",
        "text-[13px] text-field-foreground shadow-[var(--field-shadow)] outline-none transition-[color,box-shadow,border-color]",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "data-[placeholder]:text-field-placeholder",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0",
        size === "sm" ? "h-7 px-2 text-xs" : "h-8 px-2.5",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <Icon name="chevron-down" className="size-3 shrink-0 text-muted" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

// WHAT:  The floating list. `item-aligned` is the default: the list opens *over*
//        the trigger with the current option sitting on it, the way a native
//        desktop menu does, rather than as a panel dropped below.
// WHY:   Most of these selects live in dense toolbars and inspector rows where
//        a panel below has nowhere to go and flips above instead — so the same
//        control opened in two different directions depending on scroll
//        position. Aligning to the selected item makes the current value stay
//        put and every list behave the same wherever it sits.
// HOW:   Pass `position="popper"` for the anchored-below behaviour when a
//        caller genuinely wants it (a trigger at the very edge of the window).
export function SelectContent({ className, children, position = "item-aligned", ...props }: React.ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        data-slot="select-content"
        position={position}
        className={cn(
          "relative z-50 max-h-(--radix-select-content-available-height) min-w-32 origin-(--radix-select-content-transform-origin)",
          "overflow-x-hidden overflow-y-auto rounded-xl glass-modal text-popover-foreground",
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          position === "popper" ? "data-[side=bottom]:translate-y-1 data-[side=top]:-translate-y-1" : "",
          className,
        )}
        {...props}
      >
        <SelectScrollUpButton />
        <SelectPrimitive.Viewport className={cn("p-1", position === "popper" ? "h-(--radix-select-trigger-height) w-full min-w-(--radix-select-trigger-width)" : "")}>
          {children}
        </SelectPrimitive.Viewport>
        <SelectScrollDownButton />
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export function SelectItem({ className, children, ...props }: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        "relative flex w-full cursor-default select-none items-center gap-2 rounded-md py-1.5 pr-7 pl-2 text-[13px] outline-none",
        "focus:bg-surface-secondary focus:text-foreground",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0",
        className,
      )}
      {...props}
    >
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <span className="absolute right-2 flex size-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <Icon name="check" className="size-3.5 text-accent" />
        </SelectPrimitive.ItemIndicator>
      </span>
    </SelectPrimitive.Item>
  );
}

export function SelectLabel({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.Label>) {
  return <SelectPrimitive.Label data-slot="select-label" className={cn("px-2 py-1 text-[11px] font-medium text-muted", className)} {...props} />;
}

export function SelectSeparator({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.Separator>) {
  return <SelectPrimitive.Separator data-slot="select-separator" className={cn("-mx-1 my-1 h-px bg-separator", className)} {...props} />;
}

function SelectScrollUpButton({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.ScrollUpButton>) {
  return (
    <SelectPrimitive.ScrollUpButton data-slot="select-scroll-up-button" className={cn("flex cursor-default items-center justify-center py-1", className)} {...props}>
      <Icon name="chevron-down" size={14} className="rotate-180 text-muted" />
    </SelectPrimitive.ScrollUpButton>
  );
}

function SelectScrollDownButton({ className, ...props }: React.ComponentProps<typeof SelectPrimitive.ScrollDownButton>) {
  return (
    <SelectPrimitive.ScrollDownButton data-slot="select-scroll-down-button" className={cn("flex cursor-default items-center justify-center py-1", className)} {...props}>
      <Icon name="chevron-down" className="size-3.5 text-muted" />
    </SelectPrimitive.ScrollDownButton>
  );
}
