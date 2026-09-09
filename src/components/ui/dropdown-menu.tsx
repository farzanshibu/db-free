// SOT: dropdown-menu, menu, context-menu-surface
import type * as React from "react";
import { DropdownMenu as MenuPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui DropdownMenu (Radix): the app's only menu surface, used
//        both by trigger buttons and — through an invisible anchor — by
//        right-click menus.
// WHERE: https://ui.shadcn.com/docs/components/dropdown-menu
export const DropdownMenu = MenuPrimitive.Root;
export const DropdownMenuTrigger = MenuPrimitive.Trigger;
export const DropdownMenuGroup = MenuPrimitive.Group;
export const DropdownMenuPortal = MenuPrimitive.Portal;
export const DropdownMenuSub = MenuPrimitive.Sub;
export const DropdownMenuRadioGroup = MenuPrimitive.RadioGroup;

export function DropdownMenuContent({ className, sideOffset = 6, align = "start", ...props }: React.ComponentProps<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        data-slot="dropdown-menu-content"
        sideOffset={sideOffset}
        align={align}
        className={cn(
          "z-50 max-h-(--radix-dropdown-menu-content-available-height) min-w-32 overflow-y-auto overflow-x-hidden",
          "origin-(--radix-dropdown-menu-content-transform-origin) rounded-xl glass-modal p-1 text-popover-foreground",
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          className,
        )}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

export interface DropdownMenuItemProps extends React.ComponentProps<typeof MenuPrimitive.Item> {
  /// Destructive entries (drop, delete, close all) read in the danger colour.
  variant?: "default" | "danger";
  inset?: boolean;
}

export function DropdownMenuItem({ className, variant = "default", inset = false, ...props }: DropdownMenuItemProps) {
  return (
    <MenuPrimitive.Item
      data-slot="dropdown-menu-item"
      data-variant={variant}
      data-inset={inset}
      className={cn(
        "relative flex cursor-default select-none items-center gap-2 rounded-md px-2 py-1.5 text-[13px] outline-none",
        "focus:bg-surface-secondary focus:text-foreground",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
        "data-[inset=true]:pl-7",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
        variant === "danger" ? "text-danger focus:bg-danger/15 focus:text-danger [&_svg]:text-danger" : "text-foreground",
        className,
      )}
      {...props}
    />
  );
}

export function DropdownMenuCheckboxItem({ className, children, ...props }: React.ComponentProps<typeof MenuPrimitive.CheckboxItem>) {
  return (
    <MenuPrimitive.CheckboxItem
      data-slot="dropdown-menu-checkbox-item"
      className={cn(
        "relative flex cursor-default select-none items-center gap-2 rounded-md py-1.5 pr-2 pl-7 text-[13px] outline-none",
        "focus:bg-surface-secondary data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
        className,
      )}
      {...props}
    >
      <span className="pointer-events-none absolute left-2 flex size-3.5 items-center justify-center">
        <MenuPrimitive.ItemIndicator>
          <Icon name="check" className="size-3.5 text-accent" />
        </MenuPrimitive.ItemIndicator>
      </span>
      {children}
    </MenuPrimitive.CheckboxItem>
  );
}

export function DropdownMenuRadioItem({ className, children, ...props }: React.ComponentProps<typeof MenuPrimitive.RadioItem>) {
  return (
    <MenuPrimitive.RadioItem
      data-slot="dropdown-menu-radio-item"
      className={cn(
        "relative flex cursor-default select-none items-center gap-2 rounded-md py-1.5 pr-2 pl-7 text-[13px] outline-none",
        "focus:bg-surface-secondary data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
        className,
      )}
      {...props}
    >
      <span className="pointer-events-none absolute left-2 flex size-3.5 items-center justify-center">
        <MenuPrimitive.ItemIndicator>
          <span className="size-1.5 rounded-full bg-accent" />
        </MenuPrimitive.ItemIndicator>
      </span>
      {children}
    </MenuPrimitive.RadioItem>
  );
}

export function DropdownMenuLabel({ className, ...props }: React.ComponentProps<typeof MenuPrimitive.Label>) {
  return <MenuPrimitive.Label data-slot="dropdown-menu-label" className={cn("px-2 py-1 text-[11px] font-medium text-muted", className)} {...props} />;
}

export function DropdownMenuSeparator({ className, ...props }: React.ComponentProps<typeof MenuPrimitive.Separator>) {
  return <MenuPrimitive.Separator data-slot="dropdown-menu-separator" className={cn("-mx-1 my-1 h-px bg-separator", className)} {...props} />;
}

export function DropdownMenuShortcut({ className, ...props }: React.ComponentProps<"span">) {
  return <span data-slot="dropdown-menu-shortcut" className={cn("ml-auto text-[11px] tracking-widest text-muted", className)} {...props} />;
}

export function DropdownMenuSubTrigger({ className, inset = false, children, ...props }: React.ComponentProps<typeof MenuPrimitive.SubTrigger> & { inset?: boolean }) {
  return (
    <MenuPrimitive.SubTrigger
      data-slot="dropdown-menu-sub-trigger"
      data-inset={inset}
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-md px-2 py-1.5 text-[13px] outline-none",
        "focus:bg-surface-secondary data-[state=open]:bg-surface-secondary data-[inset=true]:pl-7",
        className,
      )}
      {...props}
    >
      {children}
      <Icon name="chevron-right" className="ml-auto size-3.5 text-muted" />
    </MenuPrimitive.SubTrigger>
  );
}

export function DropdownMenuSubContent({ className, ...props }: React.ComponentProps<typeof MenuPrimitive.SubContent>) {
  return (
    <MenuPrimitive.SubContent
      data-slot="dropdown-menu-sub-content"
      className={cn(
        "z-50 min-w-32 origin-(--radix-dropdown-menu-content-transform-origin) overflow-hidden rounded-xl glass-modal p-1",
        "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
        className,
      )}
      {...props}
    />
  );
}
