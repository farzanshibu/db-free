// SOT: palette-library-actions, palette-documents, palette-pins-recents, palette-run-workflow
import { useMemo } from "react";
import type { Document, DocumentKind, HomeEntity, HomeItemKind, HomePin, HomeRecent } from "@/lib/bindings";
import { HOME_KINDS, connectionEntity, documentEntity, entityKey, tableEntity } from "@/lib/home";
import type { IconName } from "@/lib/icons";
import { relativeTime } from "@/lib/relativeTime";
import { confirmRunWorkflow } from "@/features/documents/runWorkflow";
import { openHomeEntity } from "@/features/home/openEntity";
import { resolveEntity } from "@/features/home/useResolvedEntity";
import { useRecents } from "@/stores/recents";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  What the library adds to the ⌘K palette: pinned and recent items, every
//        dashboard / workflow / diagram (open it, or run a workflow), and quick pin /
//        unpin of the connection or tab you are on.
// WHY:   Jumping to "the Nightly load workflow" or "that table I had open" is what the
//        palette is for; the lists live in stores the palette does not own.
// HOW:   Entries use the palette sections that already exist for their kind
//        ("tables", "saved_queries", "dashboards", …) and are pushed ahead of the
//        generic entries, so pinned items lead their section. `useLibraryPaletteState`
//        hands the palette one object that changes when any source list does.
// WHERE: src/features/palette/CommandPalette.tsx (the `lib` fences)

export interface LibraryAction {
  id: string;
  section: string;
  label: string;
  hint?: string;
  icon: IconName;
  run: () => void;
}

export interface LibraryPaletteState {
  documents: Record<DocumentKind, Document[]>;
  pins: HomePin[];
  recents: HomeRecent[];
}

export function useLibraryPaletteState(): LibraryPaletteState {
  const documents = useWorkspace((s) => s.documents);
  const pins = useWorkspace((s) => s.pins);
  const recents = useRecents((s) => s.recents);
  return useMemo(() => ({ documents, pins, recents }), [documents, pins, recents]);
}

const SECTION_OF: Record<HomeItemKind, string> = {
  connection: "connections",
  table: "tables",
  saved_query: "saved_queries",
  dashboard: "dashboards",
  workflow: "workflows",
  diagram: "diagrams",
};

const RECENTS_LISTED = 8;

// WHAT:  Pin or unpin an entity, whichever it is now.
function togglePin(entity: HomeEntity): void {
  const state = useWorkspace.getState();
  if (state.isPinned(entity.kind, entity.itemId)) void state.unpinItem(entity.kind, entity.itemId);
  else void state.pinItem(entity);
}

// WHAT:  The entity the active tab stands for, if it is a table or a document.
function activeTabEntity(): HomeEntity | null {
  const state = useWorkspace.getState();
  const tab = state.tabs.find((t) => t.id === state.activeTabId);
  if (tab === undefined) return null;
  if (tab.kind === "table") return tableEntity(tab.connectionId, tab.table);
  if (tab.kind === "document") {
    const doc = state.documents[tab.documentKind].find((d) => d.id === tab.documentId);
    return doc === undefined ? null : documentEntity(doc, tab.connectionId ?? state.activeConnectionId);
  }
  return null;
}

export function libraryActions(library: LibraryPaletteState, activeConnectionId: string | null): LibraryAction[] {
  const workspace = useWorkspace.getState();
  const lookup = { connections: workspace.connections, savedQueries: workspace.savedQueries, documents: library.documents };
  const connectionName = (id: string | null) => (id === null ? undefined : workspace.connections.find((c) => c.id === id)?.name);
  const out: LibraryAction[] = [];

  // Pinned first, then the latest recents that are not already pinned.
  const pinnedKeys = new Set(library.pins.map((p) => entityKey(p.entity)));
  for (const pin of library.pins) {
    const resolved = resolveEntity(pin.entity, lookup);
    if (resolved === null) continue;
    const where = pin.entity.kind === "connection" ? "" : ` · ${resolved.connection.name}`;
    out.push({
      id: `pinned:${entityKey(pin.entity)}`,
      section: SECTION_OF[pin.entity.kind],
      label: resolved.label,
      hint: `Pinned${where}`,
      icon: "pin",
      run: () => void openHomeEntity(pin.entity),
    });
  }
  let listed = 0;
  for (const recent of library.recents) {
    if (listed >= RECENTS_LISTED) break;
    if (pinnedKeys.has(entityKey(recent.entity))) continue;
    const resolved = resolveEntity(recent.entity, lookup);
    if (resolved === null) continue;
    listed += 1;
    const where = recent.entity.kind === "connection" ? "" : ` · ${resolved.connection.name}`;
    out.push({
      id: `recent:${entityKey(recent.entity)}`,
      section: SECTION_OF[recent.entity.kind],
      label: resolved.label,
      hint: `Recent ${relativeTime(recent.openedAt)}${where}`,
      icon: HOME_KINDS[recent.entity.kind].icon,
      run: () => void openHomeEntity(recent.entity),
    });
  }

  // Every dashboard, workflow and diagram; a workflow can also be run from here.
  const kinds: readonly DocumentKind[] = ["dashboard", "workflow", "diagram"];
  for (const kind of kinds) {
    const meta = HOME_KINDS[kind];
    for (const doc of library.documents[kind]) {
      const entity = documentEntity(doc, activeConnectionId);
      const where = connectionName(doc.connectionId) ?? meta.label.toLowerCase();
      out.push({
        id: `doc:${kind}:${doc.id}`,
        section: SECTION_OF[kind],
        label: doc.name,
        hint: where,
        icon: meta.icon,
        run: () => {
          if (entity !== null) void openHomeEntity(entity);
          else workspace.openDocument(kind, doc.id, doc.connectionId);
        },
      });
      if (kind === "workflow") {
        out.push({
          id: `run-workflow:${doc.id}`,
          section: SECTION_OF[kind],
          label: `Run workflow: ${doc.name}`,
          hint: where,
          icon: "play",
          run: () => confirmRunWorkflow(doc),
        });
      }
    }
  }

  // Quick pinning of what you are on. Resolved when chosen: the palette list is not
  // rebuilt as tabs change, so the answer must come from the moment of the click.
  if (activeConnectionId !== null) {
    const connection = workspace.connections.find((c) => c.id === activeConnectionId);
    if (connection !== undefined) {
      out.push({
        id: "pin-connection",
        section: "navigation",
        label: "Pin or unpin this connection on Home",
        hint: connection.name,
        icon: "pin",
        run: () => togglePin(connectionEntity(connection)),
      });
    }
    out.push({
      id: "pin-tab",
      section: "navigation",
      label: "Pin or unpin the current tab on Home",
      hint: "A table, dashboard, workflow or diagram",
      icon: "pin",
      run: () => {
        const entity = activeTabEntity();
        if (entity === null) useWorkspace.getState().showInfo("Open a table, dashboard, workflow or diagram first.");
        else togglePin(entity);
      },
    });
  }
  return out;
}
