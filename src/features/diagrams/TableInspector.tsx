// SOT: table-inspector, designer-table-panel, column-editor, per-table-sql-preview
import { useMemo, useState } from "react";
import type { ConnectionColor, DiagramBody, DiagramColumn, DiagramTable } from "@/lib/bindings";
import { AppSelect, Field, type Option } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Icon } from "@/lib/icons";
import type { TypeCatalog } from "./columnTypes";
import { ColorSwatches } from "./ColorSwatches";
import { addColumn, foreignKeyMarks, moveColumn, newColumn, newRelation, relationLabel, removeColumn, renameColumn, uniqueName, updateColumn, updateTable } from "./designerModel";
import { Flag, InspectorPanel, LabeledRow, Section } from "./InspectorParts";
import { SqlView, useDiagramSql } from "./SqlPreview";
import { TypePicker } from "./TypePicker";

export type Edit = (mutator: (body: DiagramBody) => DiagramBody) => void;

/// The selects cannot hold an empty value, so "nothing chosen" is this.
const NONE = "\u0000none";

interface ForeignKeyDraft {
  from: string;
  table: string;
  to: string;
}

interface TableInspectorProps {
  table: DiagramTable;
  body: DiagramBody;
  catalog: TypeCatalog;
  connectionId: string | null;
  diagramName: string;
  edit: Edit;
  onSelectRelation: (id: string) => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onClose: () => void;
}

// WHAT:  The side panel for one table: its name, schema, colour and comment; every
//        column with its type, PK / not-null / unique / auto-increment flags,
//        default and comment; the relations it takes part in; and the SQL that
//        creates it in the diagram's dialect.
// WHY:   A table has more to say than a dialog fits, and keeping the panel open
//        beside the canvas lets the user watch the card change as they type.
// HOW:   Every control edits the diagram through `edit`, which is the designer's one
//        commit path (autosave and the canvas follow). Renaming or removing a
//        column goes through designerModel so the relations that use it follow.
export function TableInspector({ table, body, catalog, connectionId, diagramName, edit, onSelectRelation, onDuplicate, onDelete, onClose }: TableInspectorProps) {
  const marks = useMemo(() => foreignKeyMarks(body).get(table.id), [body, table.id]);
  const related = body.relations.filter((r) => r.fromTable === table.id || r.toTable === table.id);
  const names = new Map(body.tables.map((t) => [t.id, t.name]));
  const { ddl, pending } = useDiagramSql(connectionId, body, table.id);
  const [draft, setDraft] = useState<ForeignKeyDraft | null>(null);
  const tableOptions: readonly Option<string>[] = body.tables.map((t) => ({ value: t.id, label: t.name === "" ? "untitled" : t.name }));
  const columnOptions = (of: DiagramTable | undefined): readonly Option<string>[] => [
    { value: NONE, label: "Choose a column" },
    ...(of?.columns ?? []).filter((c) => c.name !== "").map((c) => ({ value: c.name, label: c.name })),
  ];

  // WHAT:  Adds the relation described by the draft form, pointing at the other
  //        table's primary key when no column was chosen and it has exactly one.
  const addForeignKey = () => {
    if (!draft) return;
    const target = body.tables.find((t) => t.id === draft.table);
    if (!target) return;
    const keys = target.columns.filter((c) => c.primaryKey);
    const only = keys[0];
    const toColumn = draft.to !== "" ? draft.to : keys.length === 1 && only ? only.name : "";
    const created = newRelation(table.id, draft.from, target.id, toColumn, { from: null, to: null });
    edit((b) => ({ ...b, relations: [...b.relations, created] }));
    setDraft(null);
    onSelectRelation(created.id);
  };

  const patchColumn = (index: number, patch: Partial<DiagramColumn>) => {
    edit((b) => updateColumn(b, table.id, index, patch));
  };

  return (
    <InspectorPanel
      title="Table"
      onClose={onClose}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onDuplicate}>
            <Icon name="copy" size={13} />
            Duplicate
          </Button>
          <Button size="sm" variant="danger-soft" className="ml-auto" onClick={onDelete}>
            <Icon name="trash" size={13} />
            Delete
          </Button>
        </>
      }
    >
      <div className="grid grid-cols-2 gap-2">
        <Field
          label="Name"
          mono
          compact
          value={table.name}
          onChange={(name) => {
            edit((b) => updateTable(b, table.id, { name }));
          }}
        />
        <Field
          label="Schema"
          mono
          compact
          optional
          value={table.schema ?? ""}
          placeholder="default"
          onChange={(schema) => {
            edit((b) => updateTable(b, table.id, { schema: schema === "" ? null : schema }));
          }}
        />
      </div>
      <LabeledRow label="Header colour">
        <ColorSwatches
          value={table.color}
          resetLabel="Accent"
          onChange={(color: ConnectionColor | null) => {
            edit((b) => updateTable(b, table.id, { color }));
          }}
        />
      </LabeledRow>
      <Field
        label="Comment"
        compact
        optional
        value={table.comment ?? ""}
        onChange={(comment) => {
          edit((b) => updateTable(b, table.id, { comment: comment === "" ? null : comment }));
        }}
      />

      <Section
        title={`Columns (${table.columns.length})`}
        action={
          <Button
            size="xs"
            variant="ghost"
            onClick={() => {
              edit((b) => addColumn(b, table.id, newColumn(uniqueName(table.columns.map((c) => c.name), "column"), catalog.textType)));
            }}
          >
            <Icon name="plus" size={12} />
            Add
          </Button>
        }
      >
        <div className="flex flex-col gap-2">
          {table.columns.map((column, index) => {
            const target = marks?.get(column.name);
            return (
              <div key={index} className="flex flex-col gap-1.5 rounded-md border border-border bg-surface-secondary/40 p-2">
                <div className="flex items-center gap-1">
                  <Input
                    value={column.name}
                    aria-label={`Column ${index + 1} name`}
                    spellCheck={false}
                    className="h-6 flex-1 font-mono text-xs"
                    onChange={(event) => {
                      edit((b) => renameColumn(b, table.id, index, event.target.value));
                    }}
                  />
                  <IconButton
                    icon="arrow-up"
                    label="Move up"
                    size={12}
                    disabled={index === 0}
                    onClick={() => {
                      edit((b) => moveColumn(b, table.id, index, -1));
                    }}
                  />
                  <IconButton
                    icon="arrow-down"
                    label="Move down"
                    size={12}
                    disabled={index === table.columns.length - 1}
                    onClick={() => {
                      edit((b) => moveColumn(b, table.id, index, 1));
                    }}
                  />
                  <IconButton
                    icon="trash"
                    label="Remove column"
                    size={12}
                    onClick={() => {
                      edit((b) => removeColumn(b, table.id, index));
                    }}
                  />
                </div>
                <TypePicker
                  value={column.dataType}
                  options={catalog.types}
                  ariaLabel={`Column ${index + 1} type`}
                  onChange={(dataType) => {
                    patchColumn(index, { dataType });
                  }}
                />
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <Flag
                    label="PK"
                    title="Primary key"
                    checked={column.primaryKey}
                    onChange={(primaryKey) => {
                      patchColumn(index, primaryKey ? { primaryKey, nullable: false } : { primaryKey });
                    }}
                  />
                  <Flag
                    label="Not null"
                    checked={!column.nullable}
                    onChange={(notNull) => {
                      patchColumn(index, { nullable: !notNull });
                    }}
                  />
                  <Flag
                    label="Unique"
                    checked={column.unique}
                    onChange={(unique) => {
                      patchColumn(index, { unique });
                    }}
                  />
                  <Flag
                    label="Auto-inc"
                    title="Auto-increment"
                    checked={column.autoIncrement}
                    onChange={(autoIncrement) => {
                      patchColumn(index, { autoIncrement });
                    }}
                  />
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  <Input
                    value={column.defaultValue ?? ""}
                    placeholder="Default"
                    aria-label={`Column ${index + 1} default`}
                    spellCheck={false}
                    className="h-6 font-mono text-xs"
                    onChange={(event) => {
                      patchColumn(index, { defaultValue: event.target.value === "" ? null : event.target.value });
                    }}
                  />
                  <Input
                    value={column.comment ?? ""}
                    placeholder="Comment"
                    aria-label={`Column ${index + 1} comment`}
                    className="h-6 text-xs"
                    onChange={(event) => {
                      patchColumn(index, { comment: event.target.value === "" ? null : event.target.value });
                    }}
                  />
                </div>
                {target !== undefined ? (
                  <p className="flex items-center gap-1 text-[11px] text-accent">
                    <Icon name="link" size={11} />
                    Foreign key {target}
                  </p>
                ) : null}
              </div>
            );
          })}
          {table.columns.length === 0 ? <p className="text-xs text-muted">No columns yet.</p> : null}
        </div>
      </Section>

      <Section
        title="Relations"
        action={
          <Button
            size="xs"
            variant="ghost"
            onClick={() => {
              setDraft(draft ? null : { from: table.columns.find((c) => c.name !== "" && !c.primaryKey)?.name ?? table.columns[0]?.name ?? "", table: body.tables.find((t) => t.id !== table.id)?.id ?? table.id, to: "" });
            }}
          >
            <Icon name={draft ? "x" : "plus"} size={12} />
            {draft ? "Cancel" : "Foreign key"}
          </Button>
        }
      >
        {draft ? (
          <div className="flex flex-col gap-2 rounded-md border border-border bg-surface-secondary/40 p-2">
            <AppSelect
              label="Column"
              size="sm"
              value={draft.from === "" ? NONE : draft.from}
              options={columnOptions(table)}
              onChange={(value) => {
                setDraft({ ...draft, from: value === NONE ? "" : value });
              }}
            />
            <AppSelect
              label="References table"
              size="sm"
              value={draft.table}
              options={tableOptions}
              onChange={(value) => {
                setDraft({ ...draft, table: value, to: "" });
              }}
            />
            <AppSelect
              label="References column"
              size="sm"
              value={draft.to === "" ? NONE : draft.to}
              options={columnOptions(body.tables.find((t) => t.id === draft.table))}
              onChange={(value) => {
                setDraft({ ...draft, to: value === NONE ? "" : value });
              }}
            />
            <Button size="sm" onClick={addForeignKey}>
              <Icon name="link" size={13} />
              Add foreign key
            </Button>
          </div>
        ) : null}
        {related.length === 0 && !draft ? (
          <p className="text-xs text-muted">Drag from one table&apos;s edge to another&apos;s, or use Foreign key above.</p>
        ) : related.length === 0 ? null : (
          <ul className="flex flex-col gap-1">
            {related.map((rel) => {
              const outgoing = rel.fromTable === table.id;
              const other = names.get(outgoing ? rel.toTable : rel.fromTable) ?? "?";
              return (
                <li key={rel.id}>
                  <button
                    type="button"
                    onClick={() => {
                      onSelectRelation(rel.id);
                    }}
                    className="flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-left text-xs hover:bg-surface-secondary"
                  >
                    <Icon name={outgoing ? "chevron-right" : "chevron-left"} size={12} className="shrink-0 text-muted" />
                    <span className="truncate font-mono">{other}</span>
                    <span className="ml-auto truncate pl-2 font-mono text-[10px] text-muted">{relationLabel(rel)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </Section>

      <Section title="SQL">
        <SqlView connectionId={connectionId} ddl={ddl} pending={pending} queryTitle={`${diagramName} · ${table.name}`} maxHeight="max-h-56" />
      </Section>
    </InspectorPanel>
  );
}
