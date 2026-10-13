// SOT: library-dialogs, name-dialog, confirm-delete-dialog, folder-picker, move-to-folder-dialog, connection-picker-dialog
import { useMemo, useState, type ReactNode } from "react";
import type { LibraryScope } from "@/lib/bindings";
import { engineMeta } from "@/lib/engines";
import { errorMessage, normalizeError } from "@/lib/ipc";
import { flattenFolders, folderPath, siblingFolderNames, nameTaken } from "@/lib/libraryTree";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useLibrary } from "@/stores/library";
import { useWorkspace } from "@/stores/workspace";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  The small modal dialogs every library list uses: name something, confirm a
//        delete, pick a folder, pick a connection to copy to.
// WHY:   Saved queries, dashboards, workflows and diagrams all rename, move, copy
//        and delete the same way; one set of dialogs keeps the four lists identical.
// HOW:   Each dialog is controlled by its caller mounting it (`onClose` unmounts)
//        and runs its action itself so it can stay open and show the error when the
//        server refuses — a duplicate name, a connection deleted meanwhile.
// WHERE: src/features/documents/DocumentsPanel.tsx, src/features/queries/QueriesPanel.tsx

interface NameDialogProps {
  title: string;
  label: string;
  initial?: string;
  confirmLabel: string;
  placeholder?: string;
  /// A reason the name cannot be used (already taken…), shown before the user submits.
  problem?: (name: string) => string | null;
  onSubmit: (name: string) => Promise<void>;
  onClose: () => void;
}

// WHAT:  Ask for one name: a new folder, a rename.
export function NameDialog({ title, label, initial = "", confirmLabel, placeholder, problem, onSubmit, onClose }: NameDialogProps) {
  const [name, setName] = useState(initial);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmed = name.trim();
  const blocked = trimmed.length > 0 && trimmed !== initial.trim() ? (problem?.(trimmed) ?? null) : null;

  const submit = async () => {
    if (trimmed.length === 0 || blocked !== null) return;
    if (trimmed === initial.trim()) {
      onClose();
      return;
    }
    setPending(true);
    setError(null);
    try {
      await onSubmit(trimmed);
      onClose();
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
      setPending(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[420px]">
        <form
          className="flex min-h-0 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
          </DialogHeader>
          <DialogBody className="flex flex-col gap-2">
            <Field label={label} value={name} onChange={setName} placeholder={placeholder} autoFocus />
            {blocked !== null || error !== null ? <p className="text-xs text-danger">{blocked ?? error}</p> : null}
          </DialogBody>
          <DialogFooter>
            <Button type="button" variant="tertiary" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" pending={pending} disabled={trimmed.length === 0 || blocked !== null}>
              {confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface ConfirmDeleteDialogProps {
  title: string;
  body: ReactNode;
  confirmLabel?: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}

// WHAT:  "Are you sure?" for a delete; the danger button runs it and closes.
export function ConfirmDeleteDialog({ title, body, confirmLabel = "Delete", onConfirm, onClose }: ConfirmDeleteDialogProps) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setPending(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
      setPending(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2 text-xs text-muted">
          <div>{body}</div>
          {error !== null ? <p className="text-xs text-danger">{error}</p> : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" pending={pending} onClick={() => void run()}>
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface FolderPickerProps {
  scope: LibraryScope;
  /// The chosen folder; null is the top level.
  value: string | null;
  onChange: (folderId: string | null) => void;
  /// A folder being moved and everything under it is not a valid target.
  exclude?: string;
  className?: string;
}

// WHAT:  A list of the scope's folders (indented by depth) plus the top level, with
//        "New folder" inline: the new folder is made inside the chosen one and
//        becomes the choice.
// WHY:   Filing something should never mean leaving the dialog to make its folder.
export function FolderPicker({ scope, value, onChange, exclude, className }: FolderPickerProps) {
  const folders = useLibrary((s) => s.folders);
  const createFolder = useLibrary((s) => s.createFolder);
  const flat = useMemo(() => flattenFolders(scope, folders, exclude), [scope, folders, exclude]);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const trimmed = name.trim();
  const clash = nameTaken(siblingFolderNames(scope, value, folders), trimmed);

  const add = async () => {
    if (trimmed.length === 0 || clash) return;
    setPending(true);
    setError(null);
    try {
      const made = await createFolder(scope, value, trimmed);
      onChange(made.id);
      setName("");
      setCreating(false);
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
    } finally {
      setPending(false);
    }
  };

  const row = (selected: boolean) =>
    cn(
      "flex h-6 w-full items-center justify-start gap-2 rounded-md px-2 text-left text-xs font-normal",
      selected ? "bg-accent/15 text-accent hover:bg-accent/20" : "text-foreground hover:bg-surface-secondary/70",
    );

  return (
    <div className={cn("flex flex-col gap-2", className)}>
      <ScrollArea className="max-h-56 rounded-lg border border-border/60 bg-surface/40">
        <div className="flex flex-col gap-0.5 p-1" role="listbox" aria-label="Folder">
          <Button variant="ghost" role="option" aria-selected={value === null} className={row(value === null)} onClick={() => onChange(null)}>
            <Icon name="home" size={13} className="shrink-0" />
            <span className="truncate">Top level</span>
          </Button>
          {flat.map(({ folder, depth, path }) => (
            <Button
              key={folder.id}
              variant="ghost"
              role="option"
              aria-selected={value === folder.id}
              title={path}
              className={row(value === folder.id)}
              style={{ paddingLeft: 8 + depth * 14 }}
              onClick={() => onChange(folder.id)}
            >
              <Icon name="folder" size={13} className="shrink-0" />
              <span className="truncate">{folder.name}</span>
            </Button>
          ))}
        </div>
      </ScrollArea>
      {creating ? (
        <form
          className="flex flex-col gap-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            void add();
          }}
        >
          <div className="flex items-end gap-2">
            <Field
              compact
              label={value === null ? "New folder at the top level" : `New folder inside ${folderPath(value, folders)}`}
              value={name}
              onChange={setName}
              placeholder="Folder name"
              autoFocus
            />
            <Button type="submit" size="sm" pending={pending} disabled={trimmed.length === 0 || clash}>
              Create
            </Button>
            <Button type="button" size="sm" variant="tertiary" onClick={() => setCreating(false)}>
              Cancel
            </Button>
          </div>
          {clash || error !== null ? <p className="text-xs text-danger">{clash ? `A folder named “${trimmed}” already exists here.` : error}</p> : null}
        </form>
      ) : (
        <Button variant="tertiary" size="sm" className="w-fit" onClick={() => setCreating(true)}>
          <Icon name="plus" size={13} />
          New folder
        </Button>
      )}
    </div>
  );
}

interface MoveToFolderDialogProps {
  scope: LibraryScope;
  title: string;
  current: string | null;
  exclude?: string;
  onMove: (folderId: string | null) => Promise<void>;
  onClose: () => void;
}

// WHAT:  "Move to folder…" for an item or a folder.
export function MoveToFolderDialog({ scope, title, current, exclude, onMove, onClose }: MoveToFolderDialogProps) {
  const [target, setTarget] = useState<string | null>(current);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const move = async () => {
    setPending(true);
    setError(null);
    try {
      await onMove(target);
      onClose();
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
      setPending(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          <FolderPicker scope={scope} value={target} onChange={setTarget} {...(exclude !== undefined ? { exclude } : {})} />
          {error !== null ? <p className="text-xs text-danger">{error}</p> : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>
            Cancel
          </Button>
          <Button pending={pending} disabled={target === current} onClick={() => void move()}>
            Move
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface ConnectionPickerDialogProps {
  title: string;
  description: string;
  /// The connection the item is already on, left out of the list.
  exclude: string | null;
  onPick: (connectionId: string) => Promise<void>;
  onClose: () => void;
}

// WHAT:  "Copy to connection…": the saved connections, one click to copy.
export function ConnectionPickerDialog({ title, description, exclude, onPick, onClose }: ConnectionPickerDialogProps) {
  const connections = useWorkspace((s) => s.connections);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const targets = connections.filter((c) => c.id !== exclude);

  const pick = async (id: string) => {
    setPendingId(id);
    setError(null);
    try {
      await onPick(id);
      onClose();
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
      setPendingId(null);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          <p className="text-xs text-muted">{description}</p>
          {targets.length === 0 ? (
            <p className="text-xs text-muted">There is no other connection to copy to.</p>
          ) : (
            <ScrollArea className="max-h-64 rounded-lg border border-border/60 bg-surface/40">
              <div className="flex flex-col gap-0.5 p-1">
                {targets.map((c) => (
                  <Button
                    key={c.id}
                    variant="ghost"
                    pending={pendingId === c.id}
                    disabled={pendingId !== null}
                    className="flex h-7 w-full items-center justify-start gap-2 rounded-md px-2 text-left text-xs font-normal text-foreground"
                    onClick={() => void pick(c.id)}
                  >
                    <EngineIcon engine={c.engine} size={16} />
                    <span className="min-w-0 flex-1 truncate">{c.name}</span>
                    <span className="shrink-0 text-[11px] text-muted">{engineMeta(c.engine).label}</span>
                  </Button>
                ))}
              </div>
            </ScrollArea>
          )}
          {error !== null ? <p className="text-xs text-danger">{error}</p> : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
