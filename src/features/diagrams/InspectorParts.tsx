// SOT: inspector-parts, designer-inspector-layout, inspector-section
import type { ReactNode } from "react";
import { IconButton } from "@/components/global/Button";
import { Check } from "@/components/global/Field";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/cn";

// WHAT:  The frame and the small controls the table, relation and note inspectors
//        share: the side panel itself, a titled section, a labelled field and a
//        compact checkbox.
// WHY:   Three inspectors with one look; changing the density or the spacing is a
//        change here, not in three files.
// WHERE: src/features/diagrams/{Table,Relation,Note}Inspector.tsx

interface InspectorPanelProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
  /// Pinned under the scrolling content (Duplicate / Delete).
  footer?: ReactNode;
}

export function InspectorPanel({ title, onClose, children, footer }: InspectorPanelProps) {
  return (
    // `nokey`: React Flow ignores Backspace / Delete typed inside it, so editing a field
    // or toggling a checkbox here can never delete the table it describes.
    <aside aria-label={title} className="nokey flex h-full w-72 shrink-0 flex-col border-l border-border bg-surface">
      <div className="flex h-7 shrink-0 items-center justify-between border-b border-border px-2.5">
        <h2 className="text-xs font-semibold text-foreground">{title}</h2>
        <IconButton icon="x" label="Close inspector" onClick={onClose} />
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-2.5 p-2.5">{children}</div>
      </ScrollArea>
      {footer ? <div className="flex shrink-0 items-center gap-2 border-t border-border px-2.5 py-1.5">{footer}</div> : null}
    </aside>
  );
}

export function Section({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <h3 className="text-[10px] font-semibold tracking-wider text-muted uppercase">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  );
}

export function LabeledRow({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <Label className="text-[11px] text-muted">{label}</Label>
      {children}
    </div>
  );
}

interface FlagProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  title?: string;
}

// WHAT:  A checkbox with its text beside it, sized for a dense row of flags.
export function Flag({ checked, onChange, label, title }: FlagProps) {
  return (
    <label className="flex cursor-pointer items-center gap-1 text-[11px] text-muted select-none" title={title}>
      <Check checked={checked} onChange={onChange} label={title ?? label} />
      {label}
    </label>
  );
}
