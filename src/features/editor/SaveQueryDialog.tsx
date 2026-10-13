// SOT: save-query-dialog, saved-query-overwrite-confirm, save-as-new-query
import { useMemo, useState, type ReactNode } from "react";
import type { SavedQuery } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { diffLines, diffStat } from "./lineDiff";

interface SaveQueryDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  /// The tab's text.
  sql: string;
  /// The saved query this tab was opened from or last saved as, if it still exists.
  linked: SavedQuery | null;
  /// Room for fields another feature adds to the save form (a folder picker).
  extra?: ReactNode;
  /// Told which saved query the tab is now tied to.
  onSaved: (saved: SavedQuery) => void;
}

/// Most diff lines drawn; a rewrite of a huge script is summarised rather than rendered.
const MAX_DIFF_LINES = 400;

const tagsOf = (text: string): string[] =>
  text
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);

// WHAT:  Saves the tab's query: overwrites the saved query the tab is tied to
//        (after showing what changes) or saves it under a new name.
// WHY:   ⌘S on a tab opened from a saved query means "update that query". The
//        diff is the confirmation: an overwrite is only safe to approve if you
//        can see what it replaces. "Save as new" leaves the original alone.
// HOW:   The tab's link to its saved query lives in the query tabs' store; this
//        dialog only decides between the two writes. Versioning of the old text
//        happens in the save path itself (src-tauri/src/services/saved_queries.rs).
// WHERE: src/features/editor/QueryPane.tsx
export function SaveQueryDialog({ open, onOpenChange, connectionId, sql, linked, extra, onSaved }: SaveQueryDialogProps) {
  const saveQuery = useWorkspace((s) => s.saveQuery);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [choice, setChoice] = useState<"overwrite" | "new">("overwrite");
  const [name, setName] = useState("");
  const [tags, setTags] = useState("");
  const [busy, setBusy] = useState(false);

  // The query being overwritten; null while saving under a new name.
  const target = choice === "overwrite" ? linked : null;
  const overwrite = target !== null;
  const lines = useMemo(() => (linked === null ? [] : diffLines(linked.sql, sql)), [linked, sql]);
  const stat = diffStat(lines);
  const changed = lines.filter((l) => l.kind !== "same");
  // Only the changed lines and a line of context either side would be nicer; the
  // whole text with the changes marked is what a script of this size needs.
  const shown = lines.slice(0, MAX_DIFF_LINES);

  const save = async () => {
    setBusy(true);
    try {
      const saved =
        target !== null
          ? await saveQuery({ ...target, sql, updatedAt: "" })
          : await saveQuery({ id: "", connectionId, name: name.trim(), sql, tags: tagsOf(tags), createdAt: "", updatedAt: "" });
      onSaved(saved);
      onOpenChange(false);
      setName("");
      setTags("");
      showInfo(overwrite ? `Updated "${saved.name}".` : `Saved "${saved.name}".`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className={overwrite ? "sm:max-w-[640px]" : "sm:max-w-[440px]"}>
        <DialogHeader>
          <DialogTitle>{target !== null ? `Overwrite "${target.name}"?` : "Save query"}</DialogTitle>
          {target !== null ? <DialogDescription>The saved version is replaced with this tab's text. The earlier text stays in its version history.</DialogDescription> : null}
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          {overwrite ? (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2 text-[11px] text-muted">
                <span className="font-medium text-foreground">Changes</span>
                <span className="text-success">+{stat.added}</span>
                <span className="text-danger">−{stat.removed}</span>
              </div>
              <ScrollArea className="max-h-[40vh] rounded-lg border border-border/60 bg-surface-secondary/40">
                <pre className="selectable py-1 font-mono text-[11px] leading-5">
                  {shown.map((line, i) => (
                    <div key={i} className={cn("flex px-2", line.kind === "add" && "bg-success/10 text-success", line.kind === "del" && "bg-danger/10 text-danger", line.kind === "same" && "text-muted")}>
                      <span className="w-4 shrink-0 select-none">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : ""}</span>
                      <span className="whitespace-pre-wrap break-all">{line.text.length === 0 ? " " : line.text}</span>
                    </div>
                  ))}
                  {lines.length > MAX_DIFF_LINES ? <div className="px-2 pt-1 text-muted">… {lines.length - MAX_DIFF_LINES} more lines</div> : null}
                </pre>
              </ScrollArea>
              {changed.length === 0 ? <p className="text-xs text-muted">The text is identical to the saved query.</p> : null}
            </div>
          ) : (
            <>
              <Field label="Name" value={name} onChange={setName} placeholder="Top customers" autoFocus />
              <Field label="Tags" optional value={tags} onChange={setTags} placeholder="reports, finance" />
              {extra}
            </>
          )}
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={() => { onOpenChange(false); }}>
            Cancel
          </Button>
          {linked !== null ? (
            <Button variant="secondary" onClick={() => { setChoice(overwrite ? "new" : "overwrite"); }}>
              <Icon name={overwrite ? "plus" : "arrow-left"} size={12} />
              {overwrite ? "Save as new…" : "Back to overwrite"}
            </Button>
          ) : null}
          <Button pending={busy} disabled={!overwrite && name.trim().length === 0} onClick={() => { void save(); }}>
            {overwrite ? "Overwrite" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
