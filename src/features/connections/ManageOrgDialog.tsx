// SOT: manage-tags-folders-dialog, tag-crud-ui, folder-crud-ui, tag-colour-palette-picker
import { useState } from "react";
import type { ConnectionFolder, ConnectionTag, ConnectionTagColor } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { ConfirmDialog } from "@/components/global/ConfirmDialog";
import { Segmented } from "@/components/global/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { TAG_COLOR_ORDER, nextFreeColor, tagColorLabel, tagNameTaken, tagSolidStyle } from "./connectionTags";
import { useConnectionOrg } from "./useConnectionOrg";

type Section = "tags" | "folders";

// WHAT:  Create, rename, recolour and delete tags and folders.
// WHY:   Both are entities now; the connections page only assigns them. This is
//        the one place that edits the lists themselves.
// HOW:   Name rules (case-insensitive unique, bounded) are Rust's; the dialog
//        shows a clash as the user types for tags and shows whatever Rust
//        refuses for the rest. Deleting removes the label, never a connection.
export function ManageOrgDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const [section, setSection] = useState<Section>("tags");
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>Tags and folders</DialogTitle>
          <DialogDescription>Group connections into folders, and label them with coloured tags. A connection can have several of each.</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex min-h-[18rem] flex-col gap-2">
          <Segmented
            label="Section"
            value={section}
            options={[
              { value: "tags", label: "Tags" },
              { value: "folders", label: "Folders" },
            ]}
            onChange={setSection}
          />
          {section === "tags" ? <TagsSection /> : <FoldersSection />}
        </DialogBody>
        <DialogFooter>
          <Button variant="secondary" onClick={() => { onOpenChange(false); }}>
            Done
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ----------------------------------------------------------------------------
// Tags
// ----------------------------------------------------------------------------

function ColorSwatchGrid({ value, onChange }: { value: ConnectionTagColor; onChange: (color: ConnectionTagColor) => void }) {
  return (
    <div role="radiogroup" aria-label="Tag colour" className="grid grid-cols-7 gap-1.5">
      {TAG_COLOR_ORDER.map((color) => (
        <Button
          key={color}
          variant="ghost"
          size="sm"
          role="radio"
          aria-checked={value === color}
          aria-label={tagColorLabel(color)}
          title={tagColorLabel(color)}
          onClick={() => { onChange(color); }}
          className={cn("size-6 min-w-6 rounded-full p-0", value === color ? "ring-2 ring-ring" : "")}
        >
          <span aria-hidden className="flex size-4 items-center justify-center rounded-full text-background" style={tagSolidStyle(color)}>
            {value === color ? <Icon name="check" size={10} /> : null}
          </span>
        </Button>
      ))}
    </div>
  );
}

function SwatchButton({ color, onChange }: { color: ConnectionTagColor; onChange: (color: ConnectionTagColor) => void }) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={`Colour: ${tagColorLabel(color)}`} title="Change colour" className="size-6 shrink-0 rounded-full p-0">
          <span aria-hidden className="size-4 rounded-full" style={tagSolidStyle(color)} />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-2">
        <ColorSwatchGrid value={color} onChange={onChange} />
      </PopoverContent>
    </Popover>
  );
}

function TagsSection() {
  const tags = useConnectionOrg((s) => s.tags);
  const saveTag = useConnectionOrg((s) => s.saveTag);
  const [name, setName] = useState("");
  const [color, setColor] = useState<ConnectionTagColor | null>(null);
  const [error, setError] = useState<string | null>(null);
  const chosen = color ?? nextFreeColor(tags);
  const taken = tagNameTaken(name, tags, null);

  const add = async () => {
    if (name.trim().length === 0 || taken) return;
    setError(null);
    try {
      await saveTag(null, name, chosen);
      setName("");
      setColor(null);
    } catch (raw) {
      setError(normalizeError(raw).message);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <SwatchButton color={chosen} onChange={setColor} />
        <Input
          value={name}
          placeholder="New tag name"
          aria-label="New tag name"
          aria-invalid={taken}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") void add();
          }}
        />
        <Button onClick={() => void add()} disabled={name.trim().length === 0 || taken}>
          <Icon name="plus" size={13} />
          Add
        </Button>
      </div>
      {taken ? <p className="-mt-1.5 text-xs text-danger">A tag named “{name.trim()}” already exists.</p> : null}
      {error !== null ? <p className="-mt-1.5 text-xs text-danger">{error}</p> : null}
      <ul className="flex flex-col gap-1" aria-label="Tags">
        {tags.map((tag) => (
          <TagRow key={tag.id} tag={tag} tags={tags} />
        ))}
      </ul>
    </div>
  );
}

function TagRow({ tag, tags }: { tag: ConnectionTag; tags: readonly ConnectionTag[] }) {
  const saveTag = useConnectionOrg((s) => s.saveTag);
  const deleteTag = useConnectionOrg((s) => s.deleteTag);
  const [draft, setDraft] = useState(tag.name);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const clash = tagNameTaken(draft, tags, tag.id);

  const commit = async (next: { name?: string; color?: ConnectionTagColor }) => {
    const name = (next.name ?? draft).trim();
    if (name.length === 0 || tagNameTaken(name, tags, tag.id)) {
      setDraft(tag.name);
      return;
    }
    if (name === tag.name && (next.color ?? tag.color) === tag.color) return;
    setError(null);
    try {
      await saveTag(tag.id, name, next.color ?? tag.color);
    } catch (raw) {
      setDraft(tag.name);
      setError(normalizeError(raw).message);
    }
  };

  const remove = async () => {
    setRemoving(true);
    try {
      await deleteTag(tag.id);
      setConfirming(false);
    } catch (raw) {
      setError(normalizeError(raw).message);
      setConfirming(false);
    } finally {
      setRemoving(false);
    }
  };

  return (
    <li className="flex flex-col gap-0.5">
      <div className="flex items-center gap-2 rounded-lg px-1 py-0.5 hover:bg-surface-secondary/50">
        <SwatchButton color={tag.color} onChange={(next) => void commit({ color: next })} />
        <Input
          value={draft}
          aria-label={`Rename ${tag.name}`}
          aria-invalid={clash}
          onChange={(event) => { setDraft(event.target.value); }}
          onBlur={() => void commit({})}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") setDraft(tag.name);
          }}
          className="h-6 border-transparent bg-transparent text-xs hover:border-input focus-visible:bg-field"
        />
        {tag.isPreset ? (
          <Badge size="sm" variant="outline">
            preset
          </Badge>
        ) : null}
        <span className="w-16 shrink-0 text-right text-[11px] tabular-nums text-muted">{tag.connectionCount} in use</span>
        <Button variant="ghost" size="icon-sm" aria-label={`Delete ${tag.name}`} onClick={() => { setConfirming(true); }} className="shrink-0 text-muted hover:text-danger">
          <Icon name="trash" size={13} />
        </Button>
      </div>
      {clash ? <p className="pl-6 text-xs text-danger">Another tag already uses that name.</p> : null}
      {error !== null ? <p className="pl-6 text-xs text-danger">{error}</p> : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete tag “${tag.name}”?`}
        description={tag.connectionCount > 0 ? `It is removed from ${tag.connectionCount} connection${tag.connectionCount === 1 ? "" : "s"}. The connections themselves are kept.` : "No connection uses it."}
        confirmLabel="Delete tag"
        pending={removing}
        onConfirm={remove}
      />
    </li>
  );
}

// ----------------------------------------------------------------------------
// Folders
// ----------------------------------------------------------------------------

function FoldersSection() {
  const folders = useConnectionOrg((s) => s.folders);
  const saveFolder = useConnectionOrg((s) => s.saveFolder);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const clash = name.trim().length > 0 && folders.some((f) => f.name.toLowerCase() === name.trim().toLowerCase());

  const add = async () => {
    if (name.trim().length === 0 || clash) return;
    setError(null);
    try {
      await saveFolder(null, name);
      setName("");
    } catch (raw) {
      setError(normalizeError(raw).message);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Icon name="folder" size={14} className="ml-1.5 shrink-0 text-muted" />
        <Input
          value={name}
          placeholder="New folder name"
          aria-label="New folder name"
          aria-invalid={clash}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") void add();
          }}
        />
        <Button onClick={() => void add()} disabled={name.trim().length === 0 || clash}>
          <Icon name="plus" size={13} />
          Add
        </Button>
      </div>
      {clash ? <p className="-mt-1.5 text-xs text-danger">A folder named “{name.trim()}” already exists.</p> : null}
      {error !== null ? <p className="-mt-1.5 text-xs text-danger">{error}</p> : null}
      {folders.length === 0 ? <p className="py-3 text-center text-xs text-muted">No folders yet. A folder can be empty.</p> : null}
      <ul className="flex flex-col gap-1" aria-label="Folders">
        {folders.map((folder) => (
          <FolderRow key={folder.id} folder={folder} folders={folders} />
        ))}
      </ul>
    </div>
  );
}

function FolderRow({ folder, folders }: { folder: ConnectionFolder; folders: readonly ConnectionFolder[] }) {
  const saveFolder = useConnectionOrg((s) => s.saveFolder);
  const deleteFolder = useConnectionOrg((s) => s.deleteFolder);
  const [draft, setDraft] = useState(folder.name);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const clash = folders.some((f) => f.id !== folder.id && f.name.toLowerCase() === draft.trim().toLowerCase());

  const commit = async () => {
    const name = draft.trim();
    if (name.length === 0 || clash || name === folder.name) {
      setDraft(folder.name);
      return;
    }
    setError(null);
    try {
      await saveFolder(folder.id, name);
    } catch (raw) {
      setDraft(folder.name);
      setError(normalizeError(raw).message);
    }
  };

  const remove = async () => {
    setRemoving(true);
    try {
      await deleteFolder(folder.id);
      setConfirming(false);
    } catch (raw) {
      setError(normalizeError(raw).message);
      setConfirming(false);
    } finally {
      setRemoving(false);
    }
  };

  return (
    <li className="flex flex-col gap-0.5">
      <div className="flex items-center gap-2 rounded-lg px-1 py-0.5 hover:bg-surface-secondary/50">
        <Icon name="folder" size={14} className="ml-1.5 shrink-0 text-muted" />
        <Input
          value={draft}
          aria-label={`Rename ${folder.name}`}
          aria-invalid={clash}
          onChange={(event) => { setDraft(event.target.value); }}
          onBlur={() => void commit()}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") setDraft(folder.name);
          }}
          className="h-6 border-transparent bg-transparent text-xs hover:border-input focus-visible:bg-field"
        />
        <span className="w-24 shrink-0 text-right text-[11px] tabular-nums text-muted">
          {folder.connectionCount} connection{folder.connectionCount === 1 ? "" : "s"}
        </span>
        <Button variant="ghost" size="icon-sm" aria-label={`Delete ${folder.name}`} onClick={() => { setConfirming(true); }} className="shrink-0 text-muted hover:text-danger">
          <Icon name="trash" size={13} />
        </Button>
      </div>
      {clash ? <p className="pl-6 text-xs text-danger">Another folder already uses that name.</p> : null}
      {error !== null ? <p className="pl-6 text-xs text-danger">{error}</p> : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete folder “${folder.name}”?`}
        description={folder.connectionCount > 0 ? `${folder.connectionCount} connection${folder.connectionCount === 1 ? " is" : "s are"} taken out of it. The connections themselves are kept.` : "The folder is empty."}
        confirmLabel="Delete folder"
        pending={removing}
        onConfirm={remove}
      />
    </li>
  );
}
