// SOT: workflow-rule-editor, rule-group-builder, filter-operators
import { useId } from "react";
import type { Combinator, Rule, RuleGroup, RuleOp } from "@/lib/bindings";
import { AppSelect, Segmented } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ColumnList } from "./common";

const OPERATORS: readonly { value: RuleOp; label: string }[] = [
  { value: "eq", label: "equals" },
  { value: "neq", label: "does not equal" },
  { value: "gt", label: "greater than" },
  { value: "gte", label: "greater or equal" },
  { value: "lt", label: "less than" },
  { value: "lte", label: "less or equal" },
  { value: "contains", label: "contains" },
  { value: "starts_with", label: "starts with" },
  { value: "ends_with", label: "ends with" },
  { value: "in", label: "is one of (a, b, c)" },
  { value: "is_null", label: "is NULL" },
  { value: "is_not_null", label: "is not NULL" },
];

const COMBINATORS: readonly { value: Combinator; label: string }[] = [
  { value: "and", label: "All of" },
  { value: "or", label: "Any of" },
];

function needsValue(op: RuleOp): boolean {
  return op !== "is_null" && op !== "is_not_null";
}

// WHAT:  Edits a group of rules joined by AND / OR (Filter and Branch use it).
// HOW:   Fields are free text with the incoming column names offered as suggestions
//        (they are only known after a run). An empty group matches every row.
export function RuleEditor({ group, columns, onChange }: { group: RuleGroup; columns: readonly string[]; onChange: (next: RuleGroup) => void }) {
  const listId = useId();
  const patchRule = (index: number, partial: Partial<Rule>) => {
    onChange({ ...group, rules: group.rules.map((rule, i) => (i === index ? { ...rule, ...partial } : rule)) });
  };
  return (
    <div className="flex flex-col gap-2">
      <ColumnList id={listId} columns={columns} />
      <Segmented value={group.combinator} options={COMBINATORS} onChange={(combinator) => { onChange({ ...group, combinator }); }} label="Match" />
      {group.rules.length === 0 ? <p className="text-xs text-muted">No rules: every row passes.</p> : null}
      {group.rules.map((rule, index) => (
        <div key={index} className="flex flex-col gap-1.5 rounded-lg border border-border bg-surface-secondary/40 p-2">
          <div className="flex items-center gap-1.5">
            <Input
              list={listId}
              value={rule.field}
              placeholder="column"
              aria-label="Column"
              className="h-6 flex-1 font-mono text-xs"
              onChange={(event) => { patchRule(index, { field: event.target.value }); }}
            />
            <AppSelect ariaLabel="Operator" size="sm" className="w-44" value={rule.op} options={OPERATORS} onChange={(op) => { patchRule(index, { op }); }} />
          </div>
          <div className="flex items-center gap-1.5">
            {needsValue(rule.op) ? (
              <Input
                value={rule.value}
                placeholder={rule.op === "in" ? "a, b, c" : "value"}
                aria-label="Value"
                className="h-6 flex-1 font-mono text-xs"
                onChange={(event) => { patchRule(index, { value: event.target.value }); }}
              />
            ) : (
              <span className="flex-1 px-1 text-xs text-muted">no value needed</span>
            )}
            <IconButton icon="trash" label="Remove rule" size={12} onClick={() => { onChange({ ...group, rules: group.rules.filter((_, i) => i !== index) }); }} />
          </div>
        </div>
      ))}
      <Button variant="secondary" size="sm" className="self-start" onClick={() => { onChange({ ...group, rules: [...group.rules, { field: "", op: "eq", value: "" }] }); }}>
        <Icon name="plus" size={12} />
        Add rule
      </Button>
    </div>
  );
}
