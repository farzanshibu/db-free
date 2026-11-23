// SOT: collapsible, collapsible-primitive
import type * as React from "react";
import { Collapsible as CollapsiblePrimitive } from "radix-ui";

// WHAT:  The shadcn/ui Collapsible (Radix): an open/closed region with a trigger.
// WHY:   The base of the app's "Advanced" / "More" disclosure. Features use the
//        typed wrapper in src/components/global/Disclosure.tsx, not this directly.
// WHERE: https://ui.shadcn.com/docs/components/collapsible
export function Collapsible(props: React.ComponentProps<typeof CollapsiblePrimitive.Root>) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

export function CollapsibleTrigger(props: React.ComponentProps<typeof CollapsiblePrimitive.CollapsibleTrigger>) {
  return <CollapsiblePrimitive.CollapsibleTrigger data-slot="collapsible-trigger" {...props} />;
}

export function CollapsibleContent(props: React.ComponentProps<typeof CollapsiblePrimitive.CollapsibleContent>) {
  return <CollapsiblePrimitive.CollapsibleContent data-slot="collapsible-content" {...props} />;
}
