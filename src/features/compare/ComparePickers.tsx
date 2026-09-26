// SOT: compare-pickers, compare-connection-picker, compare-schema-picker, compare-table-picker, compare-direction-toggle, diff-status-presentation
import { useEffect } from "react";
import type { CompareDirection, ConnectionSummary, DiffStatus, TableRef } from "@/lib/bindings";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { AppSelect, Segmented, type Option } from "@/components/global/Field";
import { EngineIcon } from "@/components/global/EngineIcon";

// WHAT:  The pickers both compare tabs share: a live connection, a schema from
//        its catalog, and which side the generated script changes.
// WHY:   Schema compare and data compare pick their two sides the same way;
//        one set of pickers keeps the two tabs reading alike.
// HOW:   Only connected connections are offered — the command resolves both
//        sessions through the guard and would refuse a disconnected one.
//        A side's catalog is loaded on first pick if the sidebar has not yet.
// WHERE: src/features/compare/{SchemaCompareTab,DataCompareTab}.tsx
const EMPTY: readonly ConnectionSummary[] = [];

export function useLiveConnections(): readonly ConnectionSummary[] {
  const connections = useWorkspace((s) => s.connections);
  const sessions = useWorkspace((s) => s.sessions);
  return connections.length === 0 ? EMPTY : connections.filter((c) => sessions.includes(c.id));
}

export function ConnectionPicker({ label, value, onChange }: { label: string; value: string; onChange: (id: string) => void }) {
  const live = useLiveConnections();
  const options: Option<string>[] = live.map((c) => ({ value: c.id, label: c.name, leading: <EngineIcon engine={c.engine} size={14} /> }));
  if (!options.some((o) => o.value === value)) options.unshift({ value, label: "Not connected" });
  return <AppSelect ariaLabel={label} value={value} options={options} onChange={onChange} size="sm" className="w-48" />;
}

// WHAT:  Catalog of one side, loaded when missing.
export function useSideCatalog(connectionId: string) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const live = useWorkspace((s) => s.sessions.includes(connectionId));
  const loadCatalog = useWorkspace((s) => s.loadCatalog);
  useEffect(() => {
    if (live && catalog === undefined) void loadCatalog(connectionId);
  }, [live, catalog, connectionId, loadCatalog]);
  return catalog;
}

export function SchemaPicker({ label, connectionId, value, onChange }: { label: string; connectionId: string; value: string | null; onChange: (schema: string | null) => void }) {
  const catalog = useSideCatalog(connectionId);
  const names = (catalog?.schemas ?? []).map((s) => s.name);
  const current = value ?? names[0] ?? "";
  const options = names.length > 0 ? names.map((n) => ({ value: n, label: n })) : [{ value: current, label: current.length > 0 ? current : "—" }];
  return <AppSelect ariaLabel={label} value={current} options={options} onChange={onChange} size="sm" icon="folder" className="w-40" disabled={names.length <= 1} />;
}

// WHAT:  A table (or view) of the picked schema; None selected shows "Choose a table…".
export function TablePicker({ label, connectionId, schema, value, onChange }: { label: string; connectionId: string; schema: string | null; value: TableRef | null; onChange: (table: TableRef) => void }) {
  const catalog = useSideCatalog(connectionId);
  const schemas = catalog?.schemas ?? [];
  const home = schemas.find((s) => s.name === schema) ?? schemas[0];
  const tables: TableRef[] = (home?.tables ?? []).map((t) => ({ schema: t.schema, name: t.name }));
  const byKey = new Map(tables.map((t) => [tableKey(t), t]));
  const current = value === null ? "" : tableKey(value);
  const options: Option<string>[] = [{ value: "", label: "Choose a table…" }, ...tables.map((t): Option<string> => ({ value: tableKey(t), label: t.name, icon: "table" }))];
  if (current !== "" && !byKey.has(current) && value !== null) options.push({ value: current, label: value.name });
  return (
    <AppSelect
      ariaLabel={label}
      value={current}
      options={options}
      onChange={(key) => {
        const table = byKey.get(key);
        if (table) onChange(table);
      }}
      size="sm"
      className="w-48"
    />
  );
}

export function DirectionToggle({ value, onChange }: { value: CompareDirection; onChange: (direction: CompareDirection) => void }) {
  return (
    <Segmented
      label="Which side the script changes"
      value={value}
      onChange={onChange}
      options={[
        { value: "left_to_right", label: "Left → Right" },
        { value: "right_to_left", label: "Right → Left" },
      ]}
    />
  );
}

// WHAT:  Label and badge tone per status. "Added" / "Removed" read in the
//        chosen direction: added = the script creates it on the target side.
export const DIFF_STATUS = {
  added: { label: "Added", variant: "success" },
  removed: { label: "Removed", variant: "danger" },
  changed: { label: "Changed", variant: "warning" },
  identical: { label: "Identical", variant: "outline" },
} as const satisfies Record<DiffStatus, { label: string; variant: "success" | "danger" | "warning" | "outline" }>;

/// The connection a script from this compare runs against: the target side.
export function targetOf(direction: CompareDirection, left: string, right: string): string {
  return direction === "left_to_right" ? right : left;
}
