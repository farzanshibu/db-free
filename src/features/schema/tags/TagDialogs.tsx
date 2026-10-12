// SOT: tag-dialog, tag-create-edit-form, tag-delete-confirm, tag-name-rules
import { useState } from "react";
import type { ConnectionColor, TableTag } from "@/lib/bindings";
import { errorMessage, normalizeError } from "@/lib/ipc";
import { AppSelect, Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ColorSwatches } from "@/features/connections/ColorSwatches";
import { useTableTags } from "./useTableTags";

// WHAT:  The two small dialogs behind a tag: create / edit (name + colour preset),
//        and the confirmation before deleting one.
// WHY:   The server is the authority on names (unique per schema, case-insensitive,
//        "Untagged" reserved), but a rule the user can see before pressing Save is
//        kinder than a toast after it — so the same rules run here first, and a
//        refusal the client did not foresee is printed under the field verbatim.
// HOW:   Controlled dialogs mounted by the panel while open. Saving goes through
//        the tag store, which reloads the scope before resolving, so the list
//        behind the dialog is already current when it closes.
// WHERE: src/features/schema/tags/useTableTags.ts, src-tauri/src/store/tags.rs (the rules)

/// Mirrors `MAX_TAG_NAME_LEN` in src-tauri/src/model/tags.rs.
const NAME_LIMIT = 40;

/// Collapses whitespace the way the store does, so "  A  b " and "A b" collide here too.
function cleanName(raw: string): string {
  return raw.split(/\s+/).filter((part) => part !== "").join(" ");
}

export interface TagScopeChoice {
  /// The tag scope: the schema name, "" for engines without schemas.
  scope: string;
  label: string;
}

interface TagDialogProps {
  connectionId: string;
  database: string;
  /// Where a new tag may live; a picker shows when there are several.
  scopes: readonly TagScopeChoice[];
  initialScope: string;
  /// Every tag of the connection + database, for the duplicate-name check.
  existing: readonly TableTag[];
  /// Edit this tag; omit to create one.
  tag?: TableTag;
  onSaved: (tag: TableTag) => void;
  onClose: () => void;
}

export function TagDialog({ connectionId, database, scopes, initialScope, existing, tag, onSaved, onClose }: TagDialogProps) {
  const saveTag = useTableTags((s) => s.saveTag);
  const [name, setName] = useState(tag?.name ?? "");
  const [color, setColor] = useState<ConnectionColor>(tag?.color ?? "blue");
  const [scope, setScope] = useState(tag?.schema ?? initialScope);
  const [serverError, setServerError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const cleaned = cleanName(name);
  const clash = existing.some((t) => t.schema === scope && t.id !== tag?.id && t.name.toLowerCase() === cleaned.toLowerCase());
  const problem =
    cleaned.length > NAME_LIMIT
      ? `At most ${NAME_LIMIT} characters.`
      : cleaned.toLowerCase() === "untagged"
        ? "\"Untagged\" is the fixed group of tables without a tag."
        : clash
          ? `A tag named "${cleaned}" already exists here.`
          : null;
  const shown = problem ?? serverError;

  const submit = async () => {
    if (cleaned === "" || problem !== null) return;
    setBusy(true);
    setServerError(null);
    try {
      const saved = await saveTag({ id: tag?.id ?? null, connectionId, database, schema: scope, name: cleaned, color });
      onSaved(saved);
      onClose();
    } catch (raw) {
      setServerError(errorMessage(normalizeError(raw)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[400px]">
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{tag ? "Edit tag" : "New tag"}</DialogTitle>
            <DialogDescription>Tags group tables in the sidebar. They live in this app only; your database is not touched.</DialogDescription>
          </DialogHeader>
          <DialogBody className="flex flex-col gap-2">
            <Field label="Name" value={name} onChange={(v) => { setName(v); setServerError(null); }} autoFocus placeholder="e.g. billing" />
            {shown !== null ? <p role="alert" className="-mt-1.5 text-xs text-danger">{shown}</p> : null}
            {tag === undefined && scopes.length > 1 ? (
              <AppSelect label="Schema" value={scope} options={scopes.map((s) => ({ value: s.scope, label: s.label }))} onChange={setScope} />
            ) : null}
            <ColorSwatches value={color} onChange={(next) => { setColor(next ?? "gray"); }} />
          </DialogBody>
          <DialogFooter>
            <Button size="sm" variant="tertiary" onClick={onClose}>Cancel</Button>
            <Button size="sm" type="submit" pending={busy} disabled={cleaned === "" || problem !== null}>
              {tag ? "Save" : "Create tag"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface DeleteTagDialogProps {
  tag: TableTag;
  /// How many tables currently carry it.
  tables: number;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}

export function DeleteTagDialog({ tag, tables, onConfirm, onClose }: DeleteTagDialogProps) {
  const [busy, setBusy] = useState(false);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle>Delete tag "{tag.name}"?</DialogTitle>
          <DialogDescription>
            {tables === 0
              ? "No table carries this tag."
              : `${String(tables)} ${tables === 1 ? "table loses" : "tables lose"} this tag. The tables themselves are not touched.`}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button size="sm" variant="tertiary" onClick={onClose}>Cancel</Button>
          <Button
            size="sm"
            variant="danger"
            pending={busy}
            onClick={() => {
              setBusy(true);
              void onConfirm().finally(() => { setBusy(false); onClose(); });
            }}
          >
            Delete tag
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
