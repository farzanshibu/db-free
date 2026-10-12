// SOT: foreign-key-editor, foreign-key-rows, reference-picker
import { useEffect, useMemo } from "react";
import type { FkAction, TableRef } from "@/lib/bindings";
import { Button } from "@/components/ui/button";
import { AppSelect, Field, type Option } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { FK_ACTIONS, blankForeignKey, qualifiedLabel, type ForeignKeyDraft } from "../ddlForm";
import { describeForeignKey } from "../ddlRules";
import { MultiPick } from "./MultiPick";

// WHAT:  The foreign-key list of a table form: one card per constraint with its
//        column(s), the referenced table and column(s), and ON DELETE / ON UPDATE.
// WHY:   Create adds keys freely; Edit shows the live keys read-only (a key is
//        changed by dropping it and adding another) and lets new ones be added.
//        The referenced table list comes from the catalog the sidebar already
//        holds, and its columns from the shared column cache, so picking one
//        costs at most one `load_columns`.
// HOW:   Composite keys keep click order (local[i] references remote[i]). A key
//        that points at the table itself (a `parent_id`) is the "(this table)"
//        entry, whose columns are the form's own.
// WHERE: src/features/schema/ddlForm.ts, src-tauri/src/model/ddl.rs (ForeignKeySpec)
const SELF = "\u0000self";
const CHOOSE = "\u0000choose";

interface ForeignKeyEditorProps {
  connectionId: string;
  /// The table being created or edited (self references point here).
  own: TableRef;
  /// The form's own live column names, in order.
  columnNames: readonly string[];
  foreignKeys: ForeignKeyDraft[];
  onChange: (next: ForeignKeyDraft[]) => void;
  /// Problems by foreign key `key`.
  issues: Record<string, string>;
  /// Why keys cannot be added or dropped (Edit on an engine that cannot), or null.
  lock: string | null;
}

// WHAT:  The columns of a referenced table, loaded once into the shared cache.
function useRefColumns(connectionId: string, schema: string | null, name: string | null): string[] {
  const key = name === null ? null : `${connectionId}:${tableKey({ schema, name })}`;
  const cached = useWorkspace((s) => (key === null ? undefined : s.columnsCache[key]));
  const loadColumns = useWorkspace((s) => s.loadColumns);
  useEffect(() => {
    if (name === null || cached !== undefined) return;
    loadColumns(connectionId, { schema, name }).catch(() => undefined);
  }, [connectionId, schema, name, cached, loadColumns]);
  return useMemo(() => cached?.map((c) => c.name) ?? [], [cached]);
}

export function ForeignKeyEditor({ connectionId, own, columnNames, foreignKeys, onChange, issues, lock }: ForeignKeyEditorProps) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const tables = useMemo(() => (catalog?.schemas ?? []).flatMap((s) => s.tables.filter((t) => t.kind === "table")), [catalog]);
  const refs = useMemo(() => new Map(tables.map((t) => [tableKey({ schema: t.schema, name: t.name }), { schema: t.schema, name: t.name }])), [tables]);
  const options: Option<string>[] = useMemo(
    () => [{ value: SELF, label: "(this table)" }, ...tables.map((t) => ({ value: tableKey({ schema: t.schema, name: t.name }), label: qualifiedLabel({ schema: t.schema, name: t.name }) }))],
    [tables],
  );
  const patch = (key: string, next: Partial<ForeignKeyDraft>) => {
    onChange(foreignKeys.map((f) => (f.key === key ? { ...f, ...next } : f)));
  };

  return (
    <section className="flex flex-col gap-2" aria-label="Foreign keys">
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold tracking-wider text-muted uppercase">Foreign keys</h3>
        <span title={lock ?? undefined}>
          <Button size="sm" variant="ghost" className="text-muted" disabled={lock !== null || columnNames.length === 0} onClick={() => { onChange([...foreignKeys, blankForeignKey(columnNames)]); }}>
            <Icon name="plus" size={12} />
            Add foreign key
          </Button>
        </span>
      </div>
      {foreignKeys.length === 0 ? <p className="text-xs text-muted">{lock ?? "No foreign keys."}</p> : null}
      {foreignKeys.map((fk) =>
        fk.originalName !== null ? (
          <div key={fk.key} className={cn("flex items-center gap-2 rounded-lg border border-border/60 px-2.5 py-1.5 text-xs", fk.removed ? "opacity-55" : "")}>
            <Icon name="link" size={13} className="shrink-0 text-muted" />
            <div className="flex min-w-0 flex-1 flex-col">
              <span className={cn("truncate font-mono text-foreground", fk.removed ? "line-through" : "")}>{fk.originalName}</span>
              <span className={cn("truncate font-mono text-muted", fk.removed ? "line-through" : "")}>{describeForeignKey(fk, own)}</span>
            </div>
            {fk.removed ? (
              <Button size="xs" variant="soft" onClick={() => { patch(fk.key, { removed: false }); }}>
                Undo
              </Button>
            ) : (
              <span title={lock ?? undefined}>
                <IconButton icon="trash" label="Drop foreign key" disabled={lock !== null} onClick={() => { patch(fk.key, { removed: true }); }} />
              </span>
            )}
          </div>
        ) : (
          <NewForeignKey
            key={fk.key}
            connectionId={connectionId}
            fk={fk}
            own={own}
            columnNames={columnNames}
            options={options}
            refs={refs}
            issue={issues[fk.key]}
            onPatch={(next) => { patch(fk.key, next); }}
            onRemove={() => { onChange(foreignKeys.filter((f) => f.key !== fk.key)); }}
          />
        ),
      )}
    </section>
  );
}

interface NewForeignKeyProps {
  connectionId: string;
  fk: ForeignKeyDraft;
  own: TableRef;
  columnNames: readonly string[];
  options: Option<string>[];
  refs: ReadonlyMap<string, TableRef>;
  issue: string | undefined;
  onPatch: (next: Partial<ForeignKeyDraft>) => void;
  onRemove: () => void;
}

function NewForeignKey({ connectionId, fk, own, columnNames, options, refs, issue, onPatch, onRemove }: NewForeignKeyProps) {
  const remote = useRefColumns(connectionId, fk.refSchema, fk.refSelf || fk.refTable === "" ? null : fk.refTable);
  const remoteColumns = fk.refSelf ? columnNames : remote;
  const selected = fk.refSelf ? SELF : fk.refTable === "" ? CHOOSE : tableKey({ schema: fk.refSchema, name: fk.refTable });
  const withBlank: Option<string>[] = selected === CHOOSE ? [{ value: CHOOSE, label: "Choose a table…" }, ...options] : options;

  const pickTable = (value: string) => {
    if (value === SELF) {
      onPatch({ refSelf: true, refSchema: own.schema, refTable: own.name, refColumns: [] });
      return;
    }
    const ref = refs.get(value);
    if (ref !== undefined) onPatch({ refSelf: false, refSchema: ref.schema, refTable: ref.name, refColumns: [] });
  };
  const actionOptions: Option<FkAction>[] = FK_ACTIONS.map((a) => ({ value: a.value, label: a.label }));

  return (
    <div className={cn("flex flex-col gap-2 rounded-lg border px-2.5 py-1.5", issue !== undefined ? "border-danger/40 bg-danger/5" : "border-border/60")}>
      <div className="grid grid-cols-[minmax(7rem,1fr)_minmax(8rem,1.1fr)_minmax(9rem,1.3fr)_minmax(8rem,1.1fr)_2rem] items-end gap-2">
        <Field label="Constraint name" value={fk.name} onChange={(name) => { onPatch({ name }); }} placeholder="(automatic)" optional mono compact />
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium text-muted">Columns</span>
          <MultiPick options={columnNames} value={fk.columns} onChange={(columns) => { onPatch({ columns }); }} ariaLabel="Local columns" placeholder="Pick columns" />
        </div>
        <AppSelect label="References" size="sm" value={selected} options={withBlank} onChange={pickTable} />
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium text-muted">Referenced columns</span>
          <MultiPick options={remoteColumns} value={fk.refColumns} onChange={(refColumns) => { onPatch({ refColumns }); }} ariaLabel="Referenced columns" placeholder="Pick columns" disabled={selected === CHOOSE} />
        </div>
        <IconButton icon="trash" label="Remove foreign key" onClick={onRemove} />
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-2 text-xs text-muted">
          On delete
          <AppSelect ariaLabel="On delete" size="sm" className="w-36" value={fk.onDelete} options={actionOptions} onChange={(onDelete) => { onPatch({ onDelete }); }} />
        </div>
        <div className="flex items-center gap-2 text-xs text-muted">
          On update
          <AppSelect ariaLabel="On update" size="sm" className="w-36" value={fk.onUpdate} options={actionOptions} onChange={(onUpdate) => { onPatch({ onUpdate }); }} />
        </div>
      </div>
      {issue !== undefined ? <p className="text-[11px] text-danger">{issue}</p> : null}
    </div>
  );
}
