// SOT: workflow-node-picker, node-search, node-categories
import { useState } from "react";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { NODE_CATEGORIES, NODE_KINDS, NODE_SPECS, type NodeKind } from "./model";

interface NodePickerProps {
  onPick: (kind: NodeKind) => void;
  disabled?: boolean;
  /// The trigger button's text.
  label?: string;
}

// WHAT:  "Add node": a searchable list of every node kind, grouped by category.
// WHY:   Seventeen kinds is too many for a flat menu and too few to need a wizard;
//        typing "csv" or "email" should find the right one at once.
// HOW:   cmdk filters on the label, the description and a few keywords per kind, so
//        a search for "cron" finds Schedule. Picking closes the list and adds the
//        node where the graph has room.
export function NodePicker({ onPick, disabled = false, label = "Add node" }: NodePickerProps) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button size="sm" variant="secondary" disabled={disabled}>
          <Icon name="plus" size={12} />
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[352px] p-2">
        <Command>
          <CommandInput placeholder="Search nodes: csv, email, branch…" />
          <CommandList className="max-h-[56vh]">
            <CommandEmpty>No node matches.</CommandEmpty>
            {NODE_CATEGORIES.map((category) => {
              const kinds = NODE_KINDS.filter((kind) => NODE_SPECS[kind].category === category.id);
              if (kinds.length === 0) return null;
              return (
                <CommandGroup key={category.id} heading={category.label}>
                  {kinds.map((kind) => {
                    const spec = NODE_SPECS[kind];
                    return (
                      <CommandItem
                        key={kind}
                        value={`${spec.label} ${spec.keywords} ${spec.description}`}
                        onSelect={() => {
                          setOpen(false);
                          onPick(kind);
                        }}
                      >
                        <span className="flex size-6 shrink-0 items-center justify-center rounded-md" style={{ background: `color-mix(in oklch, var(--color-conn-${spec.color}) 20%, transparent)`, color: `var(--color-conn-${spec.color})` }}>
                          <Icon name={spec.icon} size={14} />
                        </span>
                        <span className="min-w-0">
                          <span className="block text-xs font-medium">{spec.label}</span>
                          <span className="block truncate text-[11px] text-muted">{spec.description}</span>
                        </span>
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
              );
            })}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
