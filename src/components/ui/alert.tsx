// SOT: alert, inline-banner, status-callout
import { createContext, useContext, type ComponentProps } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";
import { Icon, type IconName } from "@/lib/icons";

// WHAT:  The shadcn/ui Alert, extended with the four statuses this app reports
//        (info, success, warning, danger) and a matching indicator icon.
// WHY:   Connection failures, read-only warnings and missing-AI-key notices are
//        all inline banners; the icon has to follow the status without every
//        caller re-choosing one.
// HOW:   The variant travels through a context so `AlertIndicator` can pick its
//        own glyph, which is what makes the component a one-liner at the call site.
// WHERE: https://ui.shadcn.com/docs/components/alert
const alertVariants = cva("flex w-full items-start gap-2.5 rounded-lg border px-3 py-2.5 text-[13px]", {
  variants: {
    variant: {
      default: "border-border bg-surface text-foreground",
      success: "border-success/30 bg-success/10 text-foreground",
      warning: "border-warning/30 bg-warning/10 text-foreground",
      danger: "border-danger/30 bg-danger/10 text-foreground",
    },
  },
  defaultVariants: { variant: "default" },
});

type AlertVariant = NonNullable<VariantProps<typeof alertVariants>["variant"]>;

const AlertContext = createContext<AlertVariant>("default");

const INDICATORS = {
  default: "info",
  success: "check",
  warning: "alert",
  danger: "alert",
} satisfies Record<AlertVariant, IconName>;

const INDICATOR_TONE = {
  default: "text-accent",
  success: "text-success",
  warning: "text-warning",
  danger: "text-danger",
} satisfies Record<AlertVariant, string>;

export function Alert({ className, variant, ...props }: ComponentProps<"div"> & VariantProps<typeof alertVariants>) {
  return (
    <AlertContext.Provider value={variant ?? "default"}>
      <div data-slot="alert" role="alert" className={cn(alertVariants({ variant }), className)} {...props} />
    </AlertContext.Provider>
  );
}

// WHAT:  The status glyph. Takes no icon: it reads the Alert's variant.
export function AlertIndicator({ className }: { className?: string }) {
  const variant = useContext(AlertContext);
  return <Icon name={INDICATORS[variant]} size={16} data-slot="alert-indicator" className={cn("mt-px shrink-0", INDICATOR_TONE[variant], className)} />;
}

export function AlertContent({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="alert-content" className={cn("flex min-w-0 flex-col gap-0.5", className)} {...props} />;
}

export function AlertTitle({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="alert-title" className={cn("text-[13px] font-medium text-foreground", className)} {...props} />;
}

export function AlertDescription({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="alert-description" className={cn("text-xs text-muted", className)} {...props} />;
}
