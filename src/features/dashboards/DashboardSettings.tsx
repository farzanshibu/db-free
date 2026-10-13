// SOT: dashboard-settings-panel, variables-editor, variable-options-editor, variable-name-input
import { useState } from "react";
import type { DashboardVariable } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { AppSelect, Field } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { VariableControl } from "./VariableControl";
import { VARIABLE_TYPES, isValidVariableName, newVariable, nextVariableName, retypeVariable, sanitizeVariableName, withOptions, type VariableType } from "./variables";

// WHAT:  The dashboard's own settings, in the side panel where a tile's options
//        usually are: the title drawn over the grid and the variables (name,
//        type, current value, and the choices of a select).
// WHY:   Variables are defined once and read by many tiles. Names and types are
//        rarely touched, so they live here; the toolbar popover is for changing
//        values.
// HOW:   A name keeps to what a `{{token}}` can spell and must be unique; a
//        rename that would collide or be empty snaps back. Renaming does not
//        rewrite tile SQL: the hint below says so. Options are chips; removing
//        one clears a value that depended on it.
// WHERE: ./DashboardTab.tsx, ./VariableControl.tsx, ./variables.ts

function NameInput({ value, taken, onCommit }: { value: string; taken: (name: string) => boolean; onCommit: (name: string) => void }) {
  const [draft, setDraft] = useState<{ source: string; text: string }>({ source: value, text: value });
  const text = draft.source === value ? draft.text : value;
  const commit = () => {
    if (text !== value && isValidVariableName(text) && !taken(text)) onCommit(text);
    else setDraft({ source: value, text: value });
  };
  return (
    <Input
      aria-label="Variable name"
      value={text}
      className="h-6 font-mono text-xs"
      aria-invalid={!isValidVariableName(text) || (text !== value && taken(text))}
      onChange={(event) => { setDraft({ source: value, text: sanitizeVariableName(event.target.value) }); }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") setDraft({ source: value, text: value });
      }}
    />
  );
}

function OptionsEditor({ options, onChange }: { options: readonly string[]; onChange: (next: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const value = draft.trim();
    if (value.length === 0 || options.includes(value)) return;
    onChange([...options, value]);
    setDraft("");
  };
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[10px] font-medium tracking-wide text-muted uppercase">Options</span>
      <div className="flex flex-wrap gap-1">
        {options.length === 0 ? <span className="text-[11px] text-muted">No options yet</span> : null}
        {options.map((option) => (
          <span key={option} className="inline-flex h-6 items-center gap-1 rounded-md bg-surface-secondary pr-1 pl-2 text-[11px] text-foreground">
            {option}
            <button type="button" aria-label={`Remove ${option}`} className="rounded text-muted hover:text-foreground" onClick={() => { onChange(options.filter((o) => o !== option)); }}>
              <Icon name="x" size={11} />
            </button>
          </span>
        ))}
      </div>
      <div className="flex items-center gap-1.5">
        <Input aria-label="New option" value={draft} placeholder="Add option…" className="h-6 flex-1 text-xs" onChange={(event) => { setDraft(event.target.value); }} onKeyDown={(event) => { if (event.key === "Enter") add(); }} />
        <Button size="icon-sm" variant="secondary" aria-label="Add option" disabled={draft.trim().length === 0} onClick={add}>
          <Icon name="plus" size={12} />
        </Button>
      </div>
    </div>
  );
}

interface SettingsProps {
  title: string;
  onTitle: (title: string) => void;
  variables: readonly DashboardVariable[];
  onVariables: (next: DashboardVariable[]) => void;
  onClose: () => void;
}

export function DashboardSettings({ title, onTitle, variables, onVariables, onClose }: SettingsProps) {
  const replace = (name: string, next: DashboardVariable) => { onVariables(variables.map((v) => (v.name === name ? next : v))); };
  return (
    <>
      <div className="flex items-center gap-2 border-b border-border/40 px-2.5 py-1.5">
        <Icon name="settings" size={14} className="text-accent" />
        <span className="text-[13px] font-semibold text-foreground">Dashboard settings</span>
        <span className="ml-auto">
          <IconButton icon="x" label="Close settings" onClick={onClose} />
        </span>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-3 p-2.5">
          <Field label="Title" value={title} onChange={onTitle} placeholder="No title" description="Drawn over the grid. Leave empty for none." />
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-foreground">Variables</span>
              <Button size="xs" variant="secondary" onClick={() => { onVariables([...variables, newVariable("text", nextVariableName(variables))]); }}>
                <Icon name="plus" size={12} />
                Add variable
              </Button>
            </div>
            <p className="text-[11px] leading-relaxed text-muted">
              Use <code className="font-mono">{"{{name}}"}</code> in a tile's SQL. Values go in as quoted literals for the connection's dialect: text <code className="font-mono">'eu'</code>, a list <code className="font-mono">'a','b'</code> (empty is <code className="font-mono">NULL</code>), a date range <code className="font-mono">{"{{name}}"}</code> as <code className="font-mono">'start' AND 'end'</code> or <code className="font-mono">{"{{name.start}}"}</code> / <code className="font-mono">{"{{name.end}}"}</code>. Renaming does not edit tile SQL.
            </p>
            {variables.length === 0 ? <p className="rounded-lg border border-dashed border-border p-2.5 text-center text-xs text-muted">No variables yet.</p> : null}
            {variables.map((v) => (
              <div key={v.name} className="flex flex-col gap-2 rounded-lg border border-border bg-background p-2">
                <div className="flex items-center gap-1.5">
                  <NameInput value={v.name} taken={(name) => variables.some((o) => o.name === name)} onCommit={(name) => { replace(v.name, { ...v, name }); }} />
                  <AppSelect<VariableType> ariaLabel="Variable type" size="sm" className="w-32 shrink-0" value={v.type} options={VARIABLE_TYPES} onChange={(type) => { if (type !== v.type) replace(v.name, retypeVariable(v, type)); }} />
                  <IconButton icon="trash" label="Remove variable" onClick={() => { onVariables(variables.filter((o) => o.name !== v.name)); }} />
                </div>
                {v.type === "select" || v.type === "multi_select" ? <OptionsEditor options={v.options} onChange={(options) => { replace(v.name, withOptions(v, options)); }} /> : null}
                <div className="flex flex-col gap-1">
                  <span className="text-[10px] font-medium tracking-wide text-muted uppercase">Value</span>
                  <VariableControl variable={v} onChange={(next) => { replace(v.name, next); }} />
                </div>
              </div>
            ))}
          </div>
        </div>
      </ScrollArea>
    </>
  );
}
