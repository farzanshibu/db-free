// SOT: connection-card, connection-card-menu, save-transient-action, copy-connection-url
import type { ConnectionSummary, ConnectionTag } from "@/lib/bindings";
import { connectionColorMeta, connectionTarget } from "@/lib/connectionGroups";
import { engineMeta } from "@/lib/engines";
import { cn } from "@/lib/cn";
import { ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { useBackupDialog } from "@/features/backup/useBackupDialog";
import { IconButton } from "@/components/global/Button";
import { EnvBadge, EnvDot } from "@/components/global/Badge";
import { useContextMenu } from "@/components/global/ContextMenu";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { connectionDetails, connectionUrl } from "./connectionUrl";
import { tagsOf } from "./connectionTags";
import { TagList } from "./TagBadge";

interface ConnectionCardProps {
  connection: ConnectionSummary;
  tags: readonly ConnectionTag[];
  menu: ReturnType<typeof useContextMenu>;
  /// Open the tags-and-folders dialog for this connection.
  onOrganize: (connection: ConnectionSummary) => void;
  /// Ask to delete (the page shows the type-the-name confirmation).
  onDelete: (connection: ConnectionSummary) => void;
  /// A tag pill was clicked: filter the list by it.
  onFilterTag: (tag: ConnectionTag) => void;
}

// WHAT:  One connection on the home page: engine, name, environment, target, tags,
//        and the actions (connect, edit, favourite, and the right-click menu).
// WHY:   A file opened for one session (`transient`) shows "Save as connection"
//        instead of Edit: there is nothing to edit until it is kept.
// HOW:   The menu mirrors DB Pro's: connect, organise, edit / duplicate, copy
//        (URL without the password, details, host or file), backup, delete.
export function ConnectionCard({ connection: c, tags, menu, onOrganize, onDelete, onFilterTag }: ConnectionCardProps) {
  const live = useWorkspace((s) => s.sessions.includes(c.id));
  const connecting = useWorkspace((s) => s.connecting === c.id);
  const select = useWorkspace((s) => s.selectConnection);
  const openForm = useWorkspace((s) => s.openForm);
  const disconnect = useWorkspace((s) => s.disconnect);
  const organize = useWorkspace((s) => s.organizeConnection);
  const refreshConnections = useWorkspace((s) => s.refreshConnections);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const openBackup = useBackupDialog((s) => s.open);
  const meta = engineMeta(c.engine);
  const target = connectionTarget(c);
  const url = connectionUrl(c);
  const transient = c.extra.transient;
  const cardTags = tagsOf(c.extra.tagIds, tags);

  const copy = (text: string, done: string) => {
    void navigator.clipboard.writeText(text).then(() => { showInfo(done); });
  };

  const saveAsConnection = async () => {
    try {
      await ipc("save_transient_connection", { id: c.id, name: null });
      await refreshConnections();
      showInfo(`Saved "${c.name}" as a connection.`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const duplicate = async () => {
    try {
      const copied = await ipc("duplicate_connection", { id: c.id });
      await refreshConnections();
      showInfo(`Created "${copied.name}".`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  return (
    <Card
      className="group relative flex min-w-0 flex-row items-center gap-2.5 overflow-hidden rounded-lg border-border/40 glass-card px-2.5 py-1.5 glass-card-hover"
      onContextMenu={(e) =>
        menu.open(
          e,
          [
            { id: "open", label: live ? "Open" : "Connect", icon: "plug" },
            { id: "disconnect", label: "Disconnect", icon: "x", disabled: !live },
            ...(transient
              ? [{ id: "save", label: "Save as connection", icon: "check" as const, group: "organize" }]
              : [
                  { id: "favorite", label: c.favorite ? "Remove from favourites" : "Add to favourites", icon: "star" as const, group: "organize" },
                  { id: "organize", label: "Tags and folders…", icon: "tag" as const, group: "organize" },
                  { id: "edit", label: "Edit connection…", icon: "pencil" as const, group: "edit" },
                  { id: "duplicate", label: "Duplicate", icon: "copy" as const, group: "edit" },
                ]),
            { id: "copy-url", label: "Copy URL (no password)", icon: "link", group: "copy", disabled: url === null },
            { id: "copy-details", label: "Copy details", icon: "clipboard", group: "copy" },
            { id: "copy-target", label: "Copy host / file", icon: "copy", group: "copy", disabled: target.length === 0 },
            ...(transient ? [] : [{ id: "backup", label: "Backup / Restore…", icon: "archive" as const, group: "copy" }]),
            ...(transient ? [] : [{ id: "delete", label: "Delete connection…", icon: "trash" as const, danger: true, group: "danger" }]),
          ],
          (action) => {
            if (action === "open") select(c.id);
            else if (action === "disconnect") void disconnect(c.id);
            else if (action === "favorite") void organize(c.id, { favorite: !c.favorite });
            else if (action === "organize") onOrganize(c);
            else if (action === "edit") openForm(c.id);
            else if (action === "duplicate") void duplicate();
            else if (action === "save") void saveAsConnection();
            else if (action === "copy-url" && url !== null) copy(url, "Connection URL copied (without the password).");
            else if (action === "copy-details") copy(connectionDetails(c), "Connection details copied.");
            else if (action === "copy-target") copy(target, "Connection target copied.");
            else if (action === "delete") onDelete(c);
            else if (action === "backup") openBackup(c.id);
          },
        )
      }
    >
      {c.color !== null ? <span aria-hidden className={cn("absolute inset-y-0 left-0 w-1", connectionColorMeta(c.color).fill)} /> : null}
      <CardContent className="flex w-full min-w-0 flex-row items-center gap-2.5 p-0">
        <span className="flex size-8 items-center justify-center overflow-hidden rounded-lg bg-surface-tertiary/70 text-accent shadow-xs border border-border/40 shrink-0">
          <EngineIcon engine={c.engine} size={20} />
        </span>
        <div className="flex min-w-0 flex-1 shrink flex-col items-start gap-0.5">
          {/* `shrink` is deliberate: the primitive ships `shrink-0` so
              buttons in a toolbar keep their size, but this one is the
              card's flexible column — without it the name and target
              keep their full width and the Connect button lands on top
              of them once the grid narrows. */}
          <Button variant="ghost" onClick={() => select(c.id)} className="h-auto w-full min-w-0 shrink flex-col items-start justify-start bg-transparent p-0 text-left hover:bg-transparent">
            <span className="flex w-full min-w-0 items-center gap-2">
              <EnvDot environment={c.environment} live={live} />
              <span className="min-w-0 truncate text-[13px] font-semibold tracking-tight text-foreground">{c.name}</span>
              {c.favorite ? <Icon name="star" size={11} className="shrink-0 text-warning" /> : null}
              <EnvBadge environment={c.environment} readOnly={c.readOnly} />
            </span>
            <span className="mt-0.5 flex w-full min-w-0 items-center gap-2 text-xs text-muted">
              <span className="shrink-0 rounded bg-surface-secondary/70 px-1.5 py-0.5 text-[10px] font-medium text-foreground/80">{meta.label}</span>
              <span className="min-w-0 truncate font-mono text-[11px] opacity-80">{target}</span>
              {live ? <span className="shrink-0 text-[11px] font-medium text-success">● connected</span> : null}
            </span>
          </Button>
          <TagList tags={cardTags} onSelect={onFilterTag} />
        </div>
        {connecting ? <Spinner size="sm" /> : null}
        <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
          {transient ? null : <IconButton icon="star" label={c.favorite ? "Remove from favourites" : "Add to favourites"} active={c.favorite} onClick={() => void organize(c.id, { favorite: !c.favorite })} />}
          {transient ? <IconButton icon="check" label="Save as connection" onClick={() => void saveAsConnection()} /> : <IconButton icon="pencil" label="Edit" onClick={() => openForm(c.id)} />}
          {live ? <IconButton icon="x" label="Disconnect" onClick={() => void disconnect(c.id)} /> : null}
        </div>
        <Button
          size="sm"
          variant={live ? "secondary" : "primary"}
          className={cn("shrink-0 rounded-md font-medium liquid-hover", live ? "glass-pill text-foreground" : "bg-accent text-accent-foreground")}
          onClick={() => select(c.id)}
        >
          {live ? "Open" : "Connect"}
        </Button>
      </CardContent>
    </Card>
  );
}
