// SOT: card, panel-card
import type * as React from "react";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Card and its slots, drawn as this app's glass panel.
// WHERE: https://ui.shadcn.com/docs/components/card
export function Card({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card" className={cn("flex flex-col rounded-xl glass-card text-card-foreground", className)} {...props} />;
}

export function CardHeader({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-header" className={cn("flex flex-col gap-1 px-4 pt-4", className)} {...props} />;
}

export function CardTitle({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-title" className={cn("text-sm font-semibold text-foreground", className)} {...props} />;
}

export function CardDescription({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-description" className={cn("text-xs text-muted", className)} {...props} />;
}

export function CardAction({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-action" className={cn("ml-auto flex items-center gap-1", className)} {...props} />;
}

export function CardContent({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-content" className={cn("p-4", className)} {...props} />;
}

export function CardFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-footer" className={cn("flex items-center gap-2 px-4 pb-4", className)} {...props} />;
}
