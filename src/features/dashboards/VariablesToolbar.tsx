// SOT: variables-toolbar, variables-popover, variable-values-row
import { useState } from "react";
import type { DashboardVariable } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { VariableControl } from "./VariableControl";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  The toolbar's Variables button: a popover with one row per variable,
//        its name on the left and the control that sets it on the right.
// WHY:   Variables are the dashboard's filters; changing one should be a click
//        or two from the canvas, without opening the settings panel.
// HOW:   A change replaces that variable in the list and goes up; the dashboard
//        re-runs the tiles whose statement it changes. "Edit variables" opens
//        the settings panel where names, types and options live.
// WHERE: ./DashboardTab.tsx, ./VariableControl.tsx, ./VariablesEditor.tsx

export function VariablesToolbar({ variables, onChange, onEdit }: { variables: readonly DashboardVariable[]; onChange: (next: DashboardVariable[]) => void; onEdit: () => void }) {
  const [open, setOpen] = useState(false);
  const set = (name: string, next: DashboardVariable) => { onChange(variables.map((v) => (v.name === name ? next : v))); };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button size="sm" variant="ghost" className="text-muted">
          <Icon name="braces" size={13} />
          Variables{variables.length > 0 ? ` (${variables.length})` : ""}
          <Icon name="chevron-down" size={12} className="opacity-60" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-0" align="end">
        <ScrollArea className="max-h-80 py-1">
          {variables.length === 0 ? (
            <p className="px-2.5 py-2.5 text-center text-xs text-muted">No variables yet. Add one to filter tiles with <code className="font-mono">{"{{name}}"}</code>.</p>
          ) : (
            variables.map((v) => (
              <div key={v.name} className="grid grid-cols-[minmax(0,6.5rem)_1fr] items-center gap-2 px-2.5 py-1">
                <span className="truncate font-mono text-[11px] text-muted" title={v.name}>{v.name}</span>
                <VariableControl variable={v} onChange={(next) => { set(v.name, next); }} />
              </div>
            ))
          )}
        </ScrollArea>
        <div className="border-t border-border/40 p-1">
          <Button size="sm" variant="ghost" className="w-full justify-start text-muted" onClick={() => { setOpen(false); onEdit(); }}>
            <Icon name="settings" size={13} />
            {variables.length === 0 ? "Add a variable" : "Edit variables"}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
