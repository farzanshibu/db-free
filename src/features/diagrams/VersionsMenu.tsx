// SOT: version-history-popover, checkpoint-popover, document-version-ui, restore-version
import { useState } from "react";
import type { Document, DocumentVersionSummary } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { relativeTime } from "./relativeTime";

interface VersionsMenuProps {
  documentId: string;
  /// Writes pending edits and files an automatic version, so the history is current before it is read.
  settle: () => Promise<void>;
  /// The document as the editor holds it now.
  current: () => Document;
  /// A version was restored; `restored` is the document as it now is.
  onRestored: (restored: Document) => void;
}

// WHAT:  The two history controls of an autosaving editor: "Save a checkpoint" (a
//        named state that is never pruned) and "History" (every saved state, with
//        a way back to any of them).
// WHY:   Autosave means there is no Save to undo, so the way back from a mistake
//        has to be somewhere else; the list is what the automatic versions are for.
// HOW:   Opening the history first settles the editor (pending edits saved, an
//        automatic version filed), so the top row is the state on screen.
//        Restoring files the state it replaces as a version first (Rust does that),
//        so a restore can itself be reversed from the same list.
// WHERE: src-tauri/src/services/document_versions.rs, src/features/diagrams/DesignerTab.tsx
export function VersionsMenu({ documentId, settle, current, onRestored }: VersionsMenuProps) {
  return (
    <div className="flex items-center gap-0.5">
      <CheckpointPopover current={current} />
      <HistoryPopover documentId={documentId} settle={settle} onRestored={onRestored} />
    </div>
  );
}

function CheckpointPopover({ current }: { current: () => Document }) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    try {
      const made = await ipc("checkpoint_document", { document: current(), name });
      showInfo(`Checkpoint “${made.name ?? "Checkpoint"}” saved.`);
      setName("");
      setOpen(false);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Save a checkpoint" className="rounded-lg liquid-hover">
              <Icon name="bookmark" size={14} />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Save a checkpoint</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="flex w-72 flex-col gap-2">
        <p className="text-xs font-semibold text-foreground">Save a checkpoint</p>
        <p className="text-[11px] leading-relaxed text-muted">A named state to come back to. Automatic versions are pruned to the latest twenty; checkpoints never are.</p>
        <Input
          autoFocus
          value={name}
          placeholder="Checkpoint name (optional)"
          aria-label="Checkpoint name"
          className="h-6.5 text-xs"
          onChange={(event) => {
            setName(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void save();
            }
          }}
        />
        <Button size="sm" disabled={saving} onClick={() => void save()}>
          {saving ? <Spinner size="sm" /> : <Icon name="check" size={13} />}
          Save checkpoint
        </Button>
      </PopoverContent>
    </Popover>
  );
}

function HistoryPopover({ documentId, settle, onRestored }: Pick<VersionsMenuProps, "documentId" | "settle" | "onRestored">) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [open, setOpen] = useState(false);
  const [versions, setVersions] = useState<DocumentVersionSummary[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);

  const load = async () => {
    try {
      await settle();
      setVersions(await ipc("list_document_versions", { documentId }));
    } catch (raw) {
      showError(normalizeError(raw));
      setVersions([]);
    }
  };

  const restore = async (version: number) => {
    setBusy(version);
    try {
      await settle();
      const restored = await ipc("restore_document_version", { documentId, version });
      onRestored(restored);
      showInfo(`Restored version ${version}.`);
      setOpen(false);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(null);
    }
  };

  const remove = async (version: number) => {
    try {
      await ipc("delete_document_version", { documentId, version });
      setVersions((current) => (current ?? []).filter((v) => v.version !== version));
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setVersions(null);
          void load();
        }
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Version history" className="rounded-lg liquid-hover">
              <Icon name="history" size={14} />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Version history</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-72 p-0">
        <div className="flex items-center gap-1.5 border-b border-border px-2.5 py-1.5">
          <Icon name="history" size={13} className="text-muted" />
          <p className="text-xs font-semibold text-foreground">Version history</p>
        </div>
        <ScrollArea className="max-h-64">
          {versions === null ? (
            <div className="flex items-center justify-center py-3">
              <Spinner size="sm" />
            </div>
          ) : versions.length === 0 ? (
            <p className="px-2.5 py-3 text-center text-xs text-muted">No versions yet. One is filed a few seconds after you stop editing.</p>
          ) : (
            versions.map((v, index) => (
              <div key={v.version} className="group flex items-start gap-2 border-b border-border px-2.5 py-1.5 last:border-b-0 hover:bg-surface-secondary/60">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center justify-between gap-2">
                    <span className={cn("truncate text-[11px] font-medium", index === 0 ? "text-accent" : "text-foreground")}>
                      {v.name ?? (index === 0 ? "Latest" : `Version ${v.version}`)}
                    </span>
                    <span className="shrink-0 text-[10px] text-muted">{relativeTime(v.createdAt)}</span>
                  </div>
                  <p className="truncate text-[10px] text-muted">{v.summary}</p>
                </div>
                <div className="flex shrink-0 items-center opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                  {index === 0 ? null : (
                    <Button size="xs" variant="soft" disabled={busy !== null} onClick={() => void restore(v.version)}>
                      {busy === v.version ? <Spinner size="sm" /> : "Restore"}
                    </Button>
                  )}
                  {v.name !== null ? <IconButton icon="trash" label="Delete checkpoint" size={12} onClick={() => void remove(v.version)} /> : null}
                </div>
              </div>
            ))
          )}
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
