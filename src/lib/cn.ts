// SOT: classname-helper, tailwind-merge
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

// WHAT:  The shadcn/ui `cn` helper: clsx for conditional classes, tailwind-merge
//        so a caller's `className` overrides a component's own utility of the
//        same family instead of losing to source order.
// WHY:   Every primitive in src/components/ui takes a `className` that has to be
//        able to beat its own cva base classes (`h-8` over `h-9`). Plain string
//        joining left both in the class list and let the stylesheet decide.
// WHERE: src/components/ui/* and every feature that restyles a primitive.
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
