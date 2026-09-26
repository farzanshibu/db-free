// SOT: schema-compare-tab, schema-diff-ui, migration-script-view
import { useMemo, useState } from "react";
import type { ColumnChange, ColumnDiff, CompareDirection, DiffStatus, ForeignKey, SchemaDiff, TableDiff } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { EmptyState } from "@/components/global/EmptyState";
import { Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ConnectionPicker, DIFF_STATUS, DirectionToggle, SchemaPicker, targetOf, useLiveConnections } from "./ComparePickers";

type Filter = "changes" | "all" | Exclude<DiffStatus, "identical">;

const FILTERS: readonly { value: Filter; label: string }[] = [
  { value: "changes", label: "Differences" },
  { value: "added", label: "Added" },
  { value: "removed", label: "Removed" },
  { value: "changed", label: "Changed" },
  { value: "all", label: "All" },
];

const STATUS_ORDER: readonly DiffStatus[] = ["added", "removed", "changed", "identical"];

function matches(filter: Filter, status: DiffStatus): boolean {
  if (filter === "all") return true;
  if (filter === "changes") return status !== "identical";
  return status === filter;
}

// WHAT:  Schema compare: pick a connection + schema per side, Compare, browse
//        the differences table by table, and take the migration script.
// WHY:   Keeping two environments in step (dev → staging → prod) is a diff of
//        catalogs plus an ALTER script; both are produced by the Rust core in
//        the target engine's dialect so the UI never writes SQL.
// HOW:   `schema_diff` resolves both sessions through the guard and returns
//        every table's status, column pairs and the script. The script is
//        read-only here; "Open in query tab" seeds an editor on the target
//        connection, where running it goes through the guarded execute path
//        (read-only lock, destructive confirmation, history).
// WHERE: src-tauri/src/services/schema_diff.rs, src-tauri/src/commands/compare.rs
export function SchemaCompareTab({ connectionId, schema }: { connectionId: string; schema: string | null }) {
  const live = useLiveConnections();
  const openQuery = useWorkspace((s) => s.openQuery);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [leftId, setLeftId] = useState(connectionId);
  const [leftSchema, setLeftSchema] = useState<string | null>(schema);
  const [rightId, setRightId] = useState(() => live.find((c) => c.id !== connectionId)?.id ?? connectionId);
  const [rightSchema, setRightSchema] = useState<string | null>(null);
  const [direction, setDirection] = useState<CompareDirection>("left_to_right");
  const [result, setResult] = useState<SchemaDiff | null>(null);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<Filter>("changes");
  const [selected, setSelected] = useState<string | null>(null);

  const leftName = live.find((c) => c.id === leftId)?.name ?? "left";
  const rightName = live.find((c) => c.id === rightId)?.name ?? "right";

  const compare = async () => {
    setBusy(true);
    try {
      const diff = await ipc("schema_diff", { left: { connectionId: leftId, schema: leftSchema }, right: { connectionId: rightId, schema: rightSchema }, direction });
      setResult(diff);
      const first = diff.tables.find((t) => t.status !== "identical");
      setSelected(first?.name ?? diff.tables[0]?.name ?? null);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  const counts = useMemo(() => {
    const out: Record<DiffStatus, number> = { added: 0, removed: 0, changed: 0, identical: 0 };
    for (const t of result?.tables ?? []) out[t.status] += 1;
    return out;
  }, [result]);
  const visible = (result?.tables ?? []).filter((t) => matches(filter, t.status));
  const current = result?.tables.find((t) => t.name === selected) ?? null;
  // The script's direction is the one the result was computed with, not the toggle's.
  const target = result ? targetOf(result.direction, leftId, rightId) : null;

  const copyScript = async () => {
    if (!result) return;
    await navigator.clipboard.writeText(result.script);
    showInfo("Migration script copied.");
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">Left</span>
        <ConnectionPicker label="Left connection" value={leftId} onChange={(id) => { setLeftId(id); setLeftSchema(null); }} />
        <SchemaPicker label="Left schema" connectionId={leftId} value={leftSchema} onChange={setLeftSchema} />
        <DirectionToggle value={direction} onChange={setDirection} />
        <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">Right</span>
        <ConnectionPicker label="Right connection" value={rightId} onChange={(id) => { setRightId(id); setRightSchema(null); }} />
        <SchemaPicker label="Right schema" connectionId={rightId} value={rightSchema} onChange={setRightSchema} />
        <Button size="sm" pending={busy} onClick={() => void compare()} className="ml-auto">
          <Icon name="git-branch" size={13} />
          Compare
        </Button>
      </div>

      {result === null ? (
        <EmptyState
          icon="git-branch"
          title="Compare two schemas"
          body={`Pick a connection and schema for each side, choose which side the migration changes, then Compare. Only connected connections are listed${live.length < 2 ? " — connect another from the sidebar to compare across databases" : ""}.`}
        />
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="flex w-[280px] shrink-0 flex-col border-r border-border/40">
            <div className="flex flex-col gap-2 p-2.5">
              <Segmented label="Filter differences" value={filter} onChange={setFilter} options={FILTERS} />
              <div className="flex flex-wrap gap-1 text-[10.5px]">
                {STATUS_ORDER.map((s) => (
                  <Badge key={s} size="sm" variant={DIFF_STATUS[s].variant} className="font-mono">
                    {counts[s]} {DIFF_STATUS[s].label.toLowerCase()}
                  </Badge>
                ))}
              </div>
            </div>
            <ScrollArea className="min-h-0 flex-1 px-1.5 pb-2">
              {visible.length === 0 ? (
                <p className="px-2 py-3 text-xs text-muted">{filter === "changes" ? "No differences: the schemas match." : "Nothing in this filter."}</p>
              ) : (
                visible.map((t) => (
                  <Button
                    key={t.name}
                    variant="ghost"
                    size="sm"
                    onClick={() => setSelected(t.name)}
                    className={cn("flex h-7.5 w-full justify-start gap-2 rounded-lg px-2 text-[12.5px]", selected === t.name ? "bg-surface-secondary text-foreground" : "text-muted")}
                  >
                    <Icon name="table" size={13} className="shrink-0 opacity-70" />
                    <span className="min-w-0 flex-1 truncate text-left">{t.name}</span>
                    <Badge size="sm" variant={DIFF_STATUS[t.status].variant}>
                      {DIFF_STATUS[t.status].label}
                    </Badge>
                  </Button>
                ))
              )}
            </ScrollArea>
          </div>

          <div className="flex min-w-0 flex-1 flex-col">
            {result.notes.length > 0 ? (
              <ul className="shrink-0 space-y-0.5 border-b border-border/40 bg-warning-soft/40 px-3 py-1.5 text-[11.5px] text-warning">
                {result.notes.map((n) => (
                  <li key={n} className="flex items-start gap-1.5">
                    <Icon name="alert" size={12} className="mt-0.5 shrink-0" />
                    {n}
                  </li>
                ))}
              </ul>
            ) : null}
            <div className="min-h-0 flex-1 overflow-auto">
              {current === null ? <EmptyState title="Pick a table" body="Select a table on the left to see its columns side by side." /> : <TableDetail table={current} leftName={leftName} rightName={rightName} />}
            </div>
            <div className="flex h-[40%] min-h-[160px] shrink-0 flex-col border-t border-border/40">
              <div className="flex h-9 shrink-0 items-center gap-2 px-3 text-xs text-muted">
                <Icon name="file" size={13} />
                <span className="font-medium text-foreground">Migration script</span>
                <span className="truncate">runs on {target === leftId ? leftName : rightName}</span>
                <Button size="xs" variant="ghost" className="ml-auto" onClick={() => void copyScript()}>
                  <Icon name="copy" size={12} />
                  Copy
                </Button>
                <Button size="xs" variant="soft" disabled={target === null} onClick={() => target !== null && openQuery(target, result.script, "migration")}>
                  <Icon name="terminal" size={12} />
                  Open in query tab
                </Button>
              </div>
              <ScrollArea className="min-h-0 flex-1 px-3 pb-3">
                <pre className="selectable rounded-xl glass-card p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-foreground">{result.script}</pre>
              </ScrollArea>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

const CHANGE_LABEL = { type: "type", nullable: "nullability", primary_key: "primary key" } as const satisfies Record<ColumnChange, string>;

// WHAT:  One table's columns, left and right side by side; a changed attribute
//        is tinted on both sides, a one-sided column on its own row.
function TableDetail({ table, leftName, rightName }: { table: TableDiff; leftName: string; rightName: string }) {
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center gap-2">
        <Icon name="table" size={14} className="text-accent" />
        <span className="text-sm font-medium text-foreground">{table.name}</span>
        <Badge size="sm" variant={DIFF_STATUS[table.status].variant}>
          {DIFF_STATUS[table.status].label}
        </Badge>
      </div>
      <table className="w-full border-collapse text-[12px]">
        <thead>
          <tr className="text-left text-[10.5px] uppercase tracking-wider text-muted">
            <th className="border-b border-border/40 px-2 py-1 font-semibold">Column</th>
            <th className="border-b border-l border-border/40 px-2 py-1 font-semibold" colSpan={3}>
              {leftName} (left)
            </th>
            <th className="border-b border-l border-border/40 px-2 py-1 font-semibold" colSpan={3}>
              {rightName} (right)
            </th>
            <th className="border-b border-l border-border/40 px-2 py-1 font-semibold">Status</th>
          </tr>
        </thead>
        <tbody>
          {table.columns.map((c) => (
            <ColumnRow key={c.name} column={c} />
          ))}
        </tbody>
      </table>
      {table.foreignKeys.length > 0 ? (
        <div className="flex flex-col gap-1">
          <span className="text-[10.5px] font-semibold uppercase tracking-wider text-muted">Foreign keys</span>
          {table.foreignKeys.map((fk, i) => {
            const key = fk.left ?? fk.right;
            return key ? (
              <div key={`${key.name}:${i}`} className="flex items-center gap-2 font-mono text-[11.5px] text-foreground">
                <Badge size="sm" variant={DIFF_STATUS[fk.status].variant}>
                  {DIFF_STATUS[fk.status].label}
                </Badge>
                <span className="text-muted">{fk.left ? "left" : "right"}</span>
                {describeKey(key)}
              </div>
            ) : null;
          })}
        </div>
      ) : null}
    </div>
  );
}

function describeKey(fk: ForeignKey): string {
  return `${fk.name}: (${fk.fromColumns.join(", ")}) → ${fk.toTable} (${fk.toColumns.join(", ")})`;
}

function ColumnRow({ column }: { column: ColumnDiff }) {
  const changed = (what: ColumnChange) => column.changes.includes(what);
  const tint = column.status === "added" ? "bg-success-soft/40" : column.status === "removed" ? "bg-danger-soft/40" : "";
  const side = (c: ColumnDiff["left"]) =>
    c === null ? (
      <td className="border-l border-border/40 px-2 py-1 text-muted/60" colSpan={3}>
        —
      </td>
    ) : (
      <>
        <td className={cn("border-l border-border/40 px-2 py-1 font-mono", changed("type") && "bg-warning-soft text-warning")}>{c.dataType}</td>
        <td className={cn("px-2 py-1 font-mono text-[11px]", changed("nullable") && "bg-warning-soft text-warning")}>{c.nullable ? "NULL" : "NOT NULL"}</td>
        <td className={cn("px-2 py-1", changed("primary_key") && "bg-warning-soft text-warning")}>{c.primaryKey ? <Icon name="key" size={11} className="text-warning" /> : null}</td>
      </>
    );
  return (
    <tr className={cn("border-b border-border/20", tint)}>
      <td className="px-2 py-1 font-medium text-foreground">{column.name}</td>
      {side(column.left)}
      {side(column.right)}
      <td className="border-l border-border/40 px-2 py-1">
        <span className="flex items-center gap-1.5">
          <Badge size="sm" variant={DIFF_STATUS[column.status].variant}>
            {DIFF_STATUS[column.status].label}
          </Badge>
          {column.changes.length > 0 ? <span className="text-[10.5px] text-muted">{column.changes.map((c) => CHANGE_LABEL[c]).join(", ")}</span> : null}
        </span>
      </td>
    </tr>
  );
}
