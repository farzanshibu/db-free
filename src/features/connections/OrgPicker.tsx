// SOT: connection-tag-picker, connection-folder-picker, org-membership-picker, inline-create-tag-folder
import { useState, type ReactNode } from "react";
import { normalizeError } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { nextFreeColor, tagNameTaken, tagSolidStyle, tagsOf, toggled } from "./connectionTags";
import { TagBadge } from "./TagBadge";
import { useConnectionOrg } from "./useConnectionOrg";
import { ScrollArea } from "@/components/ui/scroll-area";

interface Entry {
  id: string;
  name: string;
  /// A leading swatch: tags show their colour, folders an icon.
  lead: ReactNode;
}

interface PickerProps {
  label: string;
  /// "Add tag", "Add to folder"…
  addLabel: string;
  entries: readonly Entry[];
  selected: readonly string[];
  onChange: (ids: string[]) => void;
  /// Creates the entry named in the search box; returns its id. Throws with the reason on refusal.
  onCreate: (name: string) => Promise<string>;
  /// What the chips for the selected entries look like.
  chips: ReactNode;
  createNoun: string;
}

// WHAT:  The shared "pick several, or make a new one" control for tags and folders.
// WHY:   A connection holds any number of each, so the choice is a checklist in a
//        popover with search, not a select; typing a name that does not exist
//        offers to create it right there instead of sending the user to a
//        manager first.
// HOW:   Selection is controlled by the caller (the form's draft, or the
//        organise dialog's draft). Creating adds the new entry to the selection.
function Picker({ label, addLabel, entries, selected, onChange, onCreate, chips, createNoun }: PickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const needle = query.trim().toLowerCase();
  const visible = entries.filter((e) => needle.length === 0 || e.name.toLowerCase().includes(needle));
  const exact = entries.some((e) => e.name.toLowerCase() === needle);

  const create = async () => {
    setCreating(true);
    setError(null);
    try {
      const id = await onCreate(query.trim());
      onChange([...selected, id]);
      setQuery("");
    } catch (raw) {
      setError(normalizeError(raw).message);
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <Label>{label}</Label>
      <div className="flex min-h-6 flex-wrap items-center gap-1.5">
        {chips}
        <Popover
          open={open}
          onOpenChange={(next) => {
            setOpen(next);
            if (!next) {
              setQuery("");
              setError(null);
            }
          }}
        >
          <PopoverTrigger asChild>
            <Button variant="outline" size="xs" className="h-6 rounded-full border-dashed px-2 text-muted">
              <Icon name="plus" size={11} />
              {addLabel}
            </Button>
          </PopoverTrigger>
          <PopoverContent className="flex w-64 flex-col gap-2 p-2" align="start">
            <Input
              value={query}
              autoFocus
              placeholder={`Search or create a ${createNoun}…`}
              aria-label={`Search ${createNoun}s`}
              onChange={(event) => {
                setQuery(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter" && needle.length > 0 && !exact) void create();
              }}
            />
            <ScrollArea className="flex max-h-56 flex-col">
              {visible.map((entry) => (
                <Button
                  key={entry.id}
                  variant="ghost"
                  size="sm"
                  onClick={() => { onChange(toggled(selected, entry.id)); }}
                  aria-pressed={selected.includes(entry.id)}
                  className="h-6 w-full justify-start gap-2 px-1.5 text-xs font-normal text-foreground"
                >
                  <span aria-hidden className="grid size-4 shrink-0 place-items-center">
                    {selected.includes(entry.id) ? <Icon name="check" size={12} className="text-accent" /> : null}
                  </span>
                  {entry.lead}
                  <span className="min-w-0 truncate">{entry.name}</span>
                </Button>
              ))}
              {visible.length === 0 && needle.length === 0 ? <p className="px-1.5 py-1.5 text-xs text-muted">No {createNoun}s yet. Type a name to create one.</p> : null}
              {needle.length > 0 && !exact ? (
                <Button variant="ghost" size="sm" pending={creating} onClick={() => void create()} className="h-6 w-full justify-start gap-2 px-1.5 text-xs font-normal text-accent">
                  <Icon name="plus" size={12} />
                  <span className="truncate">Create “{query.trim()}”</span>
                </Button>
              ) : null}
            </ScrollArea>
            {error !== null ? <p className="px-1 text-xs text-danger">{error}</p> : null}
          </PopoverContent>
        </Popover>
      </div>
    </div>
  );
}

interface MembershipProps {
  selected: readonly string[];
  onChange: (ids: string[]) => void;
}

export function TagPicker({ selected, onChange }: MembershipProps) {
  const tags = useConnectionOrg((s) => s.tags);
  const saveTag = useConnectionOrg((s) => s.saveTag);
  const entries: Entry[] = tags.map((t) => ({ id: t.id, name: t.name, lead: <span aria-hidden className="size-2.5 shrink-0 rounded-full" style={tagSolidStyle(t.color)} /> }));
  const chips = tagsOf(selected, tags).map((tag) => <TagBadge key={tag.id} tag={tag} onRemove={() => { onChange(selected.filter((id) => id !== tag.id)); }} />);
  return (
    <Picker
      label="Tags"
      addLabel="Add tag"
      createNoun="tag"
      entries={entries}
      selected={selected}
      onChange={onChange}
      chips={chips}
      onCreate={async (name) => {
        if (tagNameTaken(name, tags, null)) throw new Error(`A tag named "${name}" already exists.`);
        const created = await saveTag(null, name, nextFreeColor(tags));
        return created.id;
      }}
    />
  );
}

export function FolderPicker({ selected, onChange }: MembershipProps) {
  const folders = useConnectionOrg((s) => s.folders);
  const saveFolder = useConnectionOrg((s) => s.saveFolder);
  const entries: Entry[] = folders.map((f) => ({ id: f.id, name: f.name, lead: <Icon name="folder" size={12} className="shrink-0 text-muted" /> }));
  const chips = folders
    .filter((f) => selected.includes(f.id))
    .map((folder) => (
      <span key={folder.id} className={cn("inline-flex h-5 max-w-[11rem] items-center gap-1 rounded-full border border-border bg-surface-secondary/60 pl-2 pr-0.5 text-[10.5px] font-medium text-foreground")}>
        <Icon name="folder" size={10} className="shrink-0 text-muted" />
        <span className="truncate">{folder.name}</span>
        <Button variant="ghost" size="xs" aria-label={`Remove from ${folder.name}`} onClick={() => { onChange(selected.filter((id) => id !== folder.id)); }} className="size-4 min-w-0 rounded-full p-0">
          <Icon name="x" size={9} />
        </Button>
      </span>
    ));
  return (
    <Picker
      label="Folders"
      addLabel="Add to folder"
      createNoun="folder"
      entries={entries}
      selected={selected}
      onChange={onChange}
      chips={chips}
      onCreate={async (name) => (await saveFolder(null, name)).id}
    />
  );
}
