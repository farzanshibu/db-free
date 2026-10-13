// SOT: pending-changes-panel, review-mode-ui, commit-flow, visual-diff, grouped-row-edits, highlighted-pending-sql, statement-comments, ddl-pending-change
import { useCallback, useEffect, useMemo, useState } from "react";
import type { ChangePreview, SqlToken, StagedChange, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { formatCell } from "@/lib/format";
import { SQL_TOKENS } from "@/lib/sqlTokens";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { useShortcut } from "@/stores/useShortcut";
import { Segmented } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Resizer } from "@/components/global/Resizer";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Shortcut } from "@/components/global/Kbd";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { commentLines, useChangeNotes } from "./changeNotes";
import { commitPending } from "./commitPending";
import { describeKey, groupChanges, type ChangeGroup } from "./pendingGroups";
import { HighlightedSql } from "./sqlHighlight";

// Stable empty list: a selector must return the same reference for unchanged state.
const EMPTY_CHANGES: StagedChange[] = [];

// WHAT:  Right-hand "Pending Changes" drawer: visual diff per row (edits to one row
//        share a card), the exact SQL with highlighting, a comment and Copy per
//        statement, Commit All (⌘S) and Clear All.
// WHY:   PRD §4.2 staged commits; the SQL comes from Rust so what you see is
//        what runs, inside one transaction. A change that matched no row stays
//        here, flagged, instead of disappearing with the ones that landed.
// HOW:   Statements are the preview's, one per staged change in order; DDL changes
//        (a rename, an index drop, a Redis `DEL`) are cards of their own and a
//        destructive one — or a destructive statement the block spots — asks first.
// WHERE: src-tauri/src/services/changes.rs, src/features/changes/commitPending.ts
export function PendingChangesPanel({ connectionId }: { connectionId: string }) {
  const changes = useWorkspace((s) => s.pendingChanges[connectionId] ?? EMPTY_CHANGES);
  const unstage = useWorkspace((s) => s.unstageChange);
  const clearChanges = useWorkspace((s) => s.clearChanges);
  const setOpen = useWorkspace((s) => s.setChangesPanelOpen);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const notes = useChangeNotes((s) => s.notes);
  const unmatched = useChangeNotes((s) => s.unmatched);
  const [view, setView] = useState<"visual" | "sql">("visual");
  const [loadedPreview, setPreview] = useState<ChangePreview | null>(null);
  // The script of the changes as they are now: while a new one is being written, the old one is not shown against them.
  const preview = changes.length === 0 || loadedPreview?.statements.length !== changes.length ? null : loadedPreview;
  const [committing, setCommitting] = useState(false);
  const [confirm, setConfirm] = useState<string[] | null>(null);
  const [width, setWidth] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("db-free:changes-panel-width");
      return saved ? Math.max(280, Math.min(750, Number(saved))) : 360;
    } catch {
      return 360;
    }
  });
  const groups = useMemo(() => groupChanges(changes), [changes]);

  const handleResize = useCallback((delta: number) => {
    setWidth((prev) => {
      // Dragging left (negative delta) expands panel width
      const next = Math.max(280, Math.min(750, prev - delta));
      try {
        localStorage.setItem("db-free:changes-panel-width", String(next));
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  useEffect(() => {
    const token = { cancelled: false };
    if (changes.length === 0) return;
    void (async () => {
      try {
        const p = await ipc("preview_changes", { connectionId, changes });
        if (!token.cancelled) setPreview(p);
      } catch (raw) {
        if (!token.cancelled) showError(normalizeError(raw));
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [changes, connectionId, showError]);

  const commit = async (confirmed: boolean) => {
    if (changes.length === 0 || committing) return;
    setCommitting(true);
    try {
      const attempt = await commitPending(connectionId, changes, confirmed);
      if (attempt.status === "confirm") setConfirm(attempt.statements);
      else setConfirm(null);
    } finally {
      setCommitting(false);
    }
  };

  // The hook keeps the latest handler in a ref, so `commit` sees current changes.
  useShortcut("commit-changes", () => void commit(false));

  // The script as it would be copied: each statement under its comment.
  const annotated = (): string => {
    if (!preview) return "";
    const lines = preview.statements.map((sql, index) => {
      const id = changes[index]?.id;
      const note = id === undefined ? undefined : notes[id];
      return `${note ? `${commentLines(note)}\n` : ""}${sql};`;
    });
    return lines.join("\n");
  };
  const copy = (text: string, what: string) => {
    void navigator.clipboard.writeText(text).then(() => showInfo(`${what} copied to the clipboard.`));
  };

  return (
    <aside className="relative flex shrink-0 flex-col glass-sidebar select-none" style={{ width }}>
      <Resizer direction="horizontal" onResize={handleResize} className="absolute -left-1 top-0 bottom-0" />
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border/40 glass-header ">
        <span className="text-[13px] font-semibold text-foreground tracking-tight">Pending Changes</span>
        <span className="ml-auto flex items-center gap-0.5">
          {view === "sql" && preview ? <IconButton icon="copy" label="Copy the script" onClick={() => copy(annotated(), "Script")} size={13} className="size-6 min-w-6" /> : null}
          <Button variant="ghost" size="icon-sm" aria-label="Hide panel" onClick={() => setOpen(false)}><Icon name="x" /></Button>
        </span>
      </div>
      <div className="px-2.5 py-1.5">
        <Segmented label="Changes view" value={view} onChange={setView} options={[{ value: "visual", label: "Visual" }, { value: "sql", label: "SQL" }]} />
      </div>
      <ScrollArea className="min-h-0 flex-1 px-2.5 pb-2">
        {changes.length === 0 ? (
          <p className="py-4 text-center text-xs text-muted">No pending changes. Double-click a cell to edit, use Insert, or select rows and Delete.</p>
        ) : view === "sql" ? (
          preview ? (
            <ul className="flex flex-col gap-2">
              {preview.statements.map((sql, index) => {
                const change = changes[index];
                if (!change) return null;
                return (
                  <li key={change.id}>
                    <StatementCard sql={sql} change={change} note={notes[change.id] ?? ""} flagged={unmatched[change.id] === true} onCopy={() => copy(`${notes[change.id] ? `${commentLines(notes[change.id] ?? "")}\n` : ""}${sql};`, "Statement")} />
                  </li>
                );
              })}
              <li className="px-1 pt-1 font-mono text-[10px] text-muted">
                {/COMMIT( TRANSACTION)?;\s*$/.test(preview.script) ? "Runs in one transaction: all of it lands or none of it." : "This engine has no transactions: each statement runs on its own."}
              </li>
            </ul>
          ) : (
            <p className="py-3 text-center text-xs text-muted">Writing the script…</p>
          )
        ) : (
          <ul className="flex flex-col gap-2">
            {groups.map((group) => (
              <li key={group.id}>
                <GroupCard group={group} flagged={group.changes.some((c) => unmatched[c.id] === true)} onUndo={() => group.changes.forEach((c) => unstage(connectionId, c.id))} />
              </li>
            ))}
          </ul>
        )}
      </ScrollArea>
      <div className="flex shrink-0 items-center gap-2 border-t border-border/40 glass-header p-2">
        <Button
          size="sm"
          variant="ghost"
          className="text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
          onClick={() => {
            useChangeNotes.getState().forget(changes.map((c) => c.id));
            clearChanges(connectionId);
          }}
          disabled={changes.length === 0}
        >
          Clear All
        </Button>
        <Button size="sm" className="flex-1 font-semibold liquid-hover" pending={committing} onClick={() => void commit(false)} disabled={changes.length === 0}>
          Commit All ({changes.length})
          <span className="ml-1 inline-flex">
            <Shortcut action="commit-changes" />
          </span>
        </Button>
      </div>

      <Dialog open={confirm !== null} onOpenChange={(open) => !open && setConfirm(null)}>
        <DialogContent className="sm:max-w-[480px]">
          <DialogHeader>
            <span className="bg-danger-soft text-danger">
              <Icon name="trash" size={16} />
            </span>
            <DialogTitle>Commit destructive changes?</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-2">
            <Alert variant="danger" className="rounded-lg">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle className="font-semibold text-xs">Destructive Operations</AlertTitle>
                <AlertDescription className="text-xs">These statements change or remove data without a safety net. Review before continuing.</AlertDescription>
              </AlertContent>
            </Alert>
            <ul className="flex flex-col gap-1.5">
              {confirm?.map((s) => (
                <li key={s} className="selectable rounded-md bg-danger-soft px-2 py-1 font-mono text-[11px] text-danger">
                  {s}
                </li>
              ))}
            </ul>
          </DialogBody>
          <DialogFooter>
            <Button variant="tertiary" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button variant="danger" onClick={() => void commit(true)}>
              Commit anyway
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
}

function FlagBadge() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge size="sm" variant="soft" color="warning" className="text-[10px]">
          matched no row
        </Badge>
      </TooltipTrigger>
      <TooltipContent>The last commit did not change anything for this row: it was probably deleted or changed elsewhere.</TooltipContent>
    </Tooltip>
  );
}

const KIND_BADGE = {
  update: { letter: "U", color: "warning" },
  insert: { letter: "I", color: "success" },
  delete: { letter: "D", color: "danger" },
  ddl: { letter: "S", color: "accent" },
} as const;

// WHAT:  One card: a row's edits (several columns share it), an insert, a delete or a statement.
function GroupCard({ group, flagged, onUndo }: { group: ChangeGroup; flagged: boolean; onUndo: () => void }) {
  const first = group.changes[0];
  if (!first) return null;
  const badge = KIND_BADGE[group.kind];
  return (
    <Card className={cn("glass-card border-border/40 p-2 shadow-xs", flagged ? "border-warning/60" : "")}>
      <CardContent className="p-0">
        <div className="flex items-center gap-2 text-xs">
          <Badge size="sm" variant="soft" color={badge.color} className="font-bold text-[10px] size-5 min-w-5 p-0 justify-center">
            {badge.letter}
          </Badge>
          <span className="truncate font-medium text-foreground">{first.kind === "ddl" && first.table.name === "" ? "Statement" : tableKey(first.table)}</span>
          {first.kind !== "ddl" ? (
            <>
              <Icon name="chevron-right" size={11} className="text-muted" />
              <span className="truncate text-muted">{first.kind === "insert" ? "new row" : describeKey(first.key, cell)}</span>
            </>
          ) : null}
          {flagged ? <FlagBadge /> : null}
          <span className="ml-auto">
            <IconButton icon="refresh" label={group.changes.length > 1 ? "Undo these changes" : "Undo this change"} onClick={onUndo} />
          </span>
        </div>
        <div className="mt-2 flex flex-col gap-1 font-mono text-[11px]">
          {group.changes.map((c) => (
            <ChangeLines key={c.id} change={c} />
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function ChangeLines({ change }: { change: StagedChange }) {
  switch (change.kind) {
    case "update":
      return (
        <div className="flex flex-col gap-1">
          <span className="truncate pl-0.5 text-[10px] uppercase tracking-wide text-muted">{change.column}</span>
          <Diff sign="-" tone="danger" text={cell(change.old)} />
          <Diff sign="+" tone="success" text={change.newRaw !== undefined ? tokenLabel(change) : cell(change.new)} />
        </div>
      );
    case "insert":
      return (
        <>
          {change.values.map((v) => (
            <Diff key={v.column} sign="+" tone="success" text={`${v.column} = ${v.raw !== undefined ? tokenText(v.raw) : cell(v.value)}`} />
          ))}
        </>
      );
    case "delete":
      return <Diff sign="-" tone="danger" text={describeKey(change.key, cell)} />;
    case "ddl":
      return (
        <div className="flex flex-col gap-1">
          {change.description.length > 0 ? <span className="font-sans text-xs text-foreground">{change.description}</span> : null}
          <HighlightedSql sql={change.statement} className="rounded bg-surface-secondary/60 px-2 py-1" />
          {change.destructive ? (
            <Badge size="sm" variant="soft" color="danger" className="self-start text-[10px]">
              destructive — asks before it runs
            </Badge>
          ) : null}
        </div>
      );
  }
}

// WHAT:  One statement of the SQL view: highlighted, with Copy and a comment.
function StatementCard({ sql, change, note, flagged, onCopy }: { sql: string; change: StagedChange; note: string; flagged: boolean; onCopy: () => void }) {
  const setNote = useChangeNotes((s) => s.set);
  const [editing, setEditing] = useState(note.length > 0);
  return (
    <Card className={cn("glass-card border-border/40 p-2 shadow-xs", flagged ? "border-warning/60" : "")}>
      <CardContent className="p-0">
        <div className="mb-1.5 flex items-center gap-1.5 text-[11px] text-muted">
          <Badge size="sm" variant="soft" color={KIND_BADGE[change.kind].color} className="font-bold text-[10px] size-5 min-w-5 p-0 justify-center">
            {KIND_BADGE[change.kind].letter}
          </Badge>
          <span className="truncate">{change.kind === "ddl" ? (change.description.length > 0 ? change.description : "Statement") : tableKey(change.table)}</span>
          {flagged ? <FlagBadge /> : null}
          <span className="ml-auto flex items-center gap-0.5">
            <IconButton icon="pencil" label={editing ? "Hide the comment" : "Add a comment"} active={editing} onClick={() => setEditing((e) => !e)} size={12} className="size-6 min-w-6" />
            <IconButton icon="copy" label="Copy this statement" onClick={onCopy} size={12} className="size-6 min-w-6" />
          </span>
        </div>
        {editing ? (
          <Textarea
            value={note}
            onChange={(e) => setNote(change.id, e.target.value)}
            placeholder="Comment — copied above this statement as -- text"
            aria-label="Comment for this statement"
            rows={2}
            className="selectable mb-1.5 min-h-0 resize-none font-mono text-[11px]"
          />
        ) : null}
        {note.length > 0 && !editing ? <HighlightedSql sql={commentLines(note)} className="mb-1" /> : null}
        <HighlightedSql sql={`${sql};`} />
      </CardContent>
    </Card>
  );
}

function Diff({ sign, tone, text }: { sign: string; tone: "danger" | "success"; text: string }) {
  return (
    <div className={cn("selectable flex gap-2 rounded px-2 py-1", tone === "danger" ? "bg-danger-soft text-danger" : "bg-success-soft text-success")}>
      <span>{sign}</span>
      <span className="truncate">{text}</span>
    </div>
  );
}

function cell(v: Value): string {
  return v.t === "null" ? "NULL" : formatCell(v).text;
}

function tokenText(token: SqlToken): string {
  return SQL_TOKENS[token].label;
}

function tokenLabel(change: Extract<StagedChange, { kind: "update" }>): string {
  return change.newRaw === undefined ? cell(change.new) : tokenText(change.newRaw);
}
