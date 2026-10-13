// SOT: fk-remap-dialog, link-column-to-table
import { useEffect, useMemo, useState } from "react";
import type { ColumnInfo, SchemaCatalog, TableRef } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { AppSelect } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cachedColumns, useFkRemaps } from "./fkStore";
import { targetColumn, type FkLink } from "./fkInference";

interface FkRemapDialogProps {
  connectionId: string;
  /// The table and column whose link is being set.
  owner: TableRef;
  column: string;
  catalog: SchemaCatalog;
  current: FkLink | undefined;
  onClose: () => void;
  onError: (message: string) => void;
}

const keyOf = (table: TableRef): string => `${table.schema ?? ""}\u0000${table.name}`;
const labelOf = (table: TableRef): string => (table.schema === null ? table.name : `${table.schema}.${table.name}`);

// WHAT:  Points a column at the table and column it really references.
// WHY:   A schema without FOREIGN KEY constraints (or one that points at a view or at
//        another schema) still has relationships; saying so once makes the cell's
//        peek, follow and value picker work.
// HOW:   Saved per connection in the app store, replacing the column's earlier link.
//        "Remove link" deletes the user's link and leaves the constraint or guess.
// WHERE: src-tauri/src/store/fk_remaps.rs
export function FkRemapDialog({ connectionId, owner, column, catalog, current, onClose, onError }: FkRemapDialogProps) {
  const tables = useMemo(() => catalog.schemas.flatMap((s) => s.tables.map((t): TableRef => ({ schema: t.schema, name: t.name }))), [catalog]);
  const save = useFkRemaps((s) => s.save);
  const remove = useFkRemaps((s) => s.remove);
  const initial = current?.table ?? tables[0];
  const [tableId, setTableId] = useState(initial ? keyOf(initial) : "");
  const [columns, setColumns] = useState<readonly ColumnInfo[] | null>(null);
  const [targetName, setTargetName] = useState(current?.column ?? "");
  const [busy, setBusy] = useState(false);
  const target = tables.find((t) => keyOf(t) === tableId);

  useEffect(() => {
    if (!target) return;
    let cancelled = false;
    void cachedColumns(connectionId, target).then((loaded) => {
      if (cancelled) return;
      setColumns(loaded);
      setTargetName((existing) => (loaded?.some((c) => c.name === existing) ? existing : (loaded ? targetColumn(loaded) : null) ?? loaded?.[0]?.name ?? ""));
    });
    return () => {
      cancelled = true;
    };
  }, [connectionId, target]);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    try {
      await action();
      onClose();
    } catch (raw) {
      onError(normalizeError(raw).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>
            Link {labelOf(owner)}.{column}
          </DialogTitle>
        </DialogHeader>
        <DialogBody>
          <div className="flex flex-col gap-2">
            <AppSelect label="References table" value={tableId} options={tables.map((t) => ({ value: keyOf(t), label: labelOf(t) }))} onChange={(next) => { setColumns(null); setTableId(next); }} />
            <AppSelect
              label="Column"
              value={targetName}
              options={(columns ?? []).map((c) => ({ value: c.name, label: c.primaryKey ? `${c.name} (primary key)` : c.name }))}
              onChange={setTargetName}
              disabled={columns === null || columns.length === 0}
            />
            {columns === null ? <p className="text-xs text-muted">Loading columns…</p> : null}
          </div>
        </DialogBody>
        <DialogFooter>
          {current?.source === "remap" && current.remapId !== undefined ? (
            <Button
              variant="ghost"
              className="mr-auto text-muted hover:text-foreground"
              disabled={busy}
              onClick={() => {
                const id = current.remapId;
                if (id !== undefined) void run(() => remove(connectionId, id));
              }}
            >
              Remove link
            </Button>
          ) : null}
          <Button variant="tertiary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={busy || !target || targetName.length === 0}
            onClick={() => {
              if (!target) return;
              void run(async () => {
                await save({ connectionId, from: owner, fromColumn: column, to: target, toColumn: targetName });
              });
            }}
          >
            Save link
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
