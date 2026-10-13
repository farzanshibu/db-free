// SOT: insert-row-modal, insert-form, required-column-validation
import { useState } from "react";
import type { CellValue, ColumnInfo, ColumnTraits } from "@/lib/bindings";
import { isRequired, traitsOf } from "@/lib/columnTraits";
import { FormValueField, type FormCell } from "@/components/global/ValueEditor";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";

interface InsertRowModalProps {
  open: boolean;
  onClose: () => void;
  columns: readonly ColumnInfo[];
  traits: readonly ColumnTraits[];
  onSubmit: (values: CellValue[]) => void;
}

// WHAT:  Insert form: one typed control per column (number field, date / time
//        picker, JSON editor, true/false select, pick-list for an ENUM, text). Empty =
//        omitted so the database default applies; the NULL toggle sends an explicit NULL.
// WHY:   The database knows which columns it fills in (auto-increment, defaults,
//        generated) and which it insists on (NOT NULL without a default); the form
//        says so per field and will not add a row that is missing a required one.
// HOW:   A blank field is left out of the INSERT. A generated column is locked and
//        never written. `now()` on a date field is the database's own current time.
// WHERE: src/components/global/ValueEditor.tsx (FormValueField), src/features/grid/TableTab.tsx
export function InsertRowModal({ open, onClose, columns, traits, onSubmit }: InsertRowModalProps) {
  const [values, setValues] = useState<Record<string, FormCell | undefined>>({});
  const missing = columns.filter((c) => isRequired(c, traitsOf(traits, c.name)) && values[c.name] === undefined);
  const [tried, setTried] = useState(false);
  const submit = () => {
    if (missing.length > 0) {
      setTried(true);
      return;
    }
    const out: CellValue[] = [];
    for (const c of columns) {
      const cell = values[c.name];
      if (cell !== undefined) out.push({ column: c.name, value: cell.value, ...(cell.raw !== undefined ? { raw: cell.raw } : {}) });
    }
    setValues({});
    setTried(false);
    onSubmit(out);
  };
  const close = () => {
    setTried(false);
    onClose();
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>Insert row</DialogTitle>
        </DialogHeader>
        <DialogBody className="max-h-[60vh] p-0">
          <ScrollArea className="max-h-[60vh] px-3 py-2">
            <div className="grid grid-cols-2 gap-2">
              {columns.map((c) => (
                <FormValueField key={c.name} column={c} traits={traitsOf(traits, c.name)} cell={values[c.name]} onChange={(next) => setValues((s) => ({ ...s, [c.name]: next }))} />
              ))}
            </div>
          </ScrollArea>
        </DialogBody>
        <DialogFooter>
          {tried && missing.length > 0 ? <span className="mr-auto text-xs text-danger">Fill in: {missing.map((c) => c.name).join(", ")}</span> : null}
          <Button variant="tertiary" onClick={close}>
            Cancel
          </Button>
          <Button onClick={submit}>Add row</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
