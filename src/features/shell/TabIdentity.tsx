// SOT: tab-identity-hook, connection-chip, tab-schema-of, tab-identity-tooltip
import { useMemo } from "react";
import { useWorkspace, type Tab } from "@/stores/workspace";
import { connectionIdentity, identityLabel, identityLines, missingIdentity, type ConnectionIdentity } from "@/lib/tabIdentity";
import { cn } from "@/lib/cn";

// WHAT:  The pieces that tie a tab to its own connection: the schema it points
//        at, the identity (colour, name, environment, database) of its
//        connection, the chip that shows it and the tooltip that spells it out.
// WHY:   A tab keeps working against the connection it was opened on even when
//        another connection is the active one, so the strip has to say which
//        that is on every tab, not once for the window.
// HOW:   Everything reads the tab's own `connectionId` — never the active
//        connection — and the live session's database when there is one.
// WHERE: src/features/shell/TabBar.tsx, src/features/shell/PaneArea.tsx,
//        src/lib/tabIdentity.ts

/// The schema / namespace a tab is looking at, for the tooltip; null where the tab has none.
export function tabSchema(tab: Tab): string | null {
  switch (tab.kind) {
    case "table":
      return tab.table.schema;
    case "erd":
    case "schema-compare":
      return tab.schema;
    case "data-compare":
      return tab.table?.schema ?? null;
    case "object":
      return tab.reference.parent;
    default:
      return null;
  }
}

/// The identity of a connection, from its record and its live session; null for no connection.
export function useConnectionIdentity(connectionId: string | null): ConnectionIdentity | null {
  const connection = useWorkspace((s) => (connectionId === null ? undefined : s.connections.find((c) => c.id === connectionId)));
  const sessionDatabase = useWorkspace((s) => (connectionId === null ? null : (s.sessionInfos[connectionId]?.database ?? null)));
  return useMemo(() => {
    if (connectionId === null) return null;
    if (connection === undefined) return missingIdentity(connectionId);
    return connectionIdentity(connection, sessionDatabase ?? connection.database);
  }, [connectionId, connection, sessionDatabase]);
}

/// The identity of the connection a tab belongs to; null for a tab that belongs to none
/// (a dashboard or diagram not tied to a connection).
export function useTabIdentity(tab: Tab | null): ConnectionIdentity | null {
  return useConnectionIdentity(tab === null ? null : tab.connectionId);
}

// WHAT:  The small "connection · database" label on a tab: a dot in the
//        connection's colour, its name, and PROD / STG / LOCAL when it has an environment.
export function ConnectionChip({ identity, className }: { identity: ConnectionIdentity; className?: string }) {
  const meta = identity.environmentMeta;
  return (
    <span
      data-slot="connection-chip"
      className={cn(
        "flex min-w-0 max-w-28 shrink items-center gap-1 rounded-sm bg-surface-tertiary/60 px-1 text-[10px] leading-4",
        identity.missing ? "text-danger" : "text-muted",
        className,
      )}
    >
      <span aria-hidden className={cn("size-1.5 shrink-0 rounded-full", identity.classes.stripe)} />
      <span className="min-w-0 truncate">{identityLabel(identity)}</span>
      {meta.short !== "" ? <span className={cn("shrink-0 font-semibold tracking-tight", meta.text)}>{meta.short}</span> : null}
    </span>
  );
}

// WHAT:  The tooltip body: the tab's name, then the full connection · database · schema.
export function TabTooltipBody({ label, identity, schema }: { label: string; identity: ConnectionIdentity | null; schema: string | null }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="font-medium text-foreground">{label}</span>
      {identity === null ? (
        <span className="text-muted">Not tied to a connection</span>
      ) : (
        identityLines(identity, schema).map((line) => (
          <span key={line} className="text-muted">
            {line}
          </span>
        ))
      )}
    </div>
  );
}
