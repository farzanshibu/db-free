// SOT: workflow-form-parts, secret-field, template-hint, node-form-context
import { useId, type ReactNode } from "react";
import type { NodeSecret, WorkflowNode } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { ConfigOf, NodeKind } from "../model";

// WHAT:  What every node form receives.
// WHY:   Forms edit one node's config and need only a little of the world around
//        it: which connection the workflow defaults to, which columns will arrive
//        (for field pickers), and a way to run the node.
export interface FormContext {
  node: WorkflowNode;
  /// The workflow's own connection id ("" when none).
  workflowConnectionId: string;
  /// Columns the node receives, read from the last run of what feeds it.
  columns: readonly string[];
  runNode: () => void;
}

export interface FormProps<K extends NodeKind> {
  config: ConfigOf<K>;
  onChange: (next: ConfigOf<K>) => void;
  context: FormContext;
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-[11px] font-semibold uppercase tracking-wide text-muted">{title}</h3>
      {children}
    </section>
  );
}

// WHAT:  The placeholder syntax, as a short reminder under a templated field.
export function TemplateHint({ children }: { children?: ReactNode }) {
  return (
    <p className="text-[11px] leading-relaxed text-muted">
      {children ?? (
        <>
          Use <code className="font-mono text-foreground">{"{{column}}"}</code> for a value from the first input row,{" "}
          <code className="font-mono text-foreground">{"{{rows.table}}"}</code> for all of them.
        </>
      )}
    </p>
  );
}

// WHAT:  A password-style field for a secret the core keeps sealed.
// WHY:   The UI never receives the secret back, only "one is set". Typing writes a
//        replacement; the saved one can be cleared; a new node that starts from the
//        last Email / Slack setup says so, and the core copies the sealed secret
//        when the workflow is saved.
export function SecretField({ label, secret, onChange, placeholder }: { label: string; secret: NodeSecret; onChange: (next: NodeSecret) => void; placeholder: string }) {
  const id = useId();
  const typed = secret.plain ?? "";
  const saved = secret.hasSecret && !secret.clear;
  const inherited = secret.inheritLast && saved && typed === "";
  return (
    <div className="flex w-full flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex items-center gap-1.5">
        <Input
          id={id}
          type="password"
          autoComplete="off"
          value={typed}
          placeholder={saved ? (inherited ? "Same as the last one you saved" : "Saved. Type to replace it") : placeholder}
          onChange={(event) => { onChange({ ...secret, plain: event.target.value, clear: false }); }}
        />
        {saved ? (
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Forget the saved ${label.toLowerCase()}`}
            onClick={() => { onChange({ ...secret, plain: null, hasSecret: false, clear: true, inheritLast: false }); }}
          >
            <Icon name="x" size={12} />
            Clear
          </Button>
        ) : null}
      </div>
      <p className="flex items-center gap-1 text-[11px] text-muted">
        <Icon name="lock" size={11} />
        {saved ? "Stored encrypted on this machine." : "Stored encrypted on this machine once you save."}
      </p>
    </div>
  );
}

/// Column names as a `<datalist>` any text input can point at with `list`.
export function ColumnList({ id, columns }: { id: string; columns: readonly string[] }) {
  return (
    <datalist id={id}>
      {columns.map((column) => (
        <option key={column} value={column} />
      ))}
    </datalist>
  );
}
