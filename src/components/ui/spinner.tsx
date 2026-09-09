// SOT: spinner, loading-indicator
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

const SIZES = { sm: 14, md: 16, lg: 24 } satisfies Record<string, number>;

export interface SpinnerProps {
  size?: keyof typeof SIZES;
  className?: string;
}

// WHAT:  The shadcn/ui Spinner, drawn with the app's own icon set.
// WHY:   Queries, connections and object loads are all slow enough to need one,
//        and every surface should show the same thing while it waits.
// WHERE: https://ui.shadcn.com/docs/components/spinner
export function Spinner({ className, size = "md" }: SpinnerProps) {
  return (
    <span role="status" aria-label="Loading" data-slot="spinner" className={cn("inline-flex text-muted", className)}>
      <Icon name="refresh" size={SIZES[size]} className="animate-spin" />
    </span>
  );
}
