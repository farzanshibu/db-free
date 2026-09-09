// SOT: button, button-variants, icon-button-base
import type * as React from "react";
import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Button, carrying this app's variant vocabulary.
// WHY:   Toolbars, panels and dialogs need eight weights of button, from a
//        borderless icon affordance to a destructive confirm. Naming them here
//        once means a feature picks an intent, never a colour.
// HOW:   cva composes base + variant + size; `asChild` renders the styles onto
//        the caller's own element (a link, a menu trigger) via Radix Slot.
// WHERE: https://ui.shadcn.com/docs/components/button
const buttonVariants = cva(
  [
    "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg",
    "text-[13px] font-medium outline-none transition-[color,background-color,border-color,box-shadow] duration-150",
    "disabled:pointer-events-none disabled:opacity-45",
    "focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:ring-offset-0",
    "[&_svg]:pointer-events-none [&_svg]:shrink-0",
  ],
  {
    variants: {
      variant: {
        /// The one affirmative action on a surface: run, connect, save.
        primary: "bg-accent text-accent-foreground shadow-xs hover:brightness-110",
        /// Paired with a primary: cancel, secondary confirm.
        secondary: "bg-surface-secondary text-foreground border border-border hover:bg-surface-tertiary",
        /// Quieter than secondary; toolbar chips and inline affordances.
        tertiary: "bg-surface/60 text-muted hover:bg-surface-secondary hover:text-foreground",
        /// Accent-tinted fill without the full weight of primary.
        soft: "bg-accent/15 text-accent hover:bg-accent/25",
        /// No chrome until hovered: icon rails, list-row actions.
        ghost: "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
        /// Chrome is the border only: pickers, empty-state calls to action.
        outline: "border border-border bg-transparent text-foreground hover:bg-surface-secondary",
        /// Irreversible: drop, delete, disconnect all.
        danger: "bg-danger text-danger-foreground shadow-xs hover:brightness-110",
        /// Destructive but not the primary action of the surface.
        "danger-soft": "bg-danger/15 text-danger hover:bg-danger/25",
        /// A dense action row: no chrome until hovered, tuned for `size="xs"`.
        toolbar: "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
        link: "text-link underline-offset-4 hover:underline",
      },
      size: {
        xs: "h-6 min-w-6 gap-1 rounded-md px-1.5 text-[11px] [&_svg:not([class*='size-'])]:size-3",
        sm: "h-7 min-w-7 gap-1 rounded-md px-2 text-xs [&_svg:not([class*='size-'])]:size-3.5",
        md: "h-8 min-w-8 px-3 [&_svg:not([class*='size-'])]:size-4",
        lg: "h-9 min-w-9 px-4 [&_svg:not([class*='size-'])]:size-4",
        icon: "size-8 px-0 [&_svg:not([class*='size-'])]:size-4",
        "icon-sm": "size-7 rounded-md px-0 [&_svg:not([class*='size-'])]:size-3.5",
      },
    },
    // A bare <Button> is the affirmative action of its surface — Save, Connect,
    // Run, Commit. That was the default this codebase was written against, and
    // dozens of call sites still say `<Button onClick={…}>Save</Button>` and mean
    // the accent-filled one.
    defaultVariants: { variant: "primary", size: "md" },
  },
);

export interface ButtonProps extends React.ComponentProps<"button">, VariantProps<typeof buttonVariants> {
  /// Render the styles onto the child element instead of a <button>.
  asChild?: boolean;
  /// An action is in flight: shows a spinner and stops further presses. The
  /// button keeps its width so a toolbar does not reflow while it works.
  pending?: boolean;
}

export function Button({ className, variant, size, asChild = false, pending = false, disabled, type = "button", children, ...props }: ButtonProps) {
  if (asChild) {
    return <Slot.Root data-slot="button" className={cn(buttonVariants({ variant, size }), className)} {...props}>{children}</Slot.Root>;
  }
  return (
    <button
      data-slot="button"
      type={type}
      data-pending={pending ? "true" : undefined}
      disabled={disabled === true || pending}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    >
      {pending ? <Icon name="refresh" aria-hidden className="animate-spin" /> : null}
      {children}
    </button>
  );
}

export { buttonVariants };
