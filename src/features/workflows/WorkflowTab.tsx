// SOT: workflow-tab, workflow-editor, workflow-autosave, workflow-run-controls, workflow-toolbar
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ReactFlowProvider, useReactFlow } from "@xyflow/react";
import type { Document, NodeConfig, WorkflowBody, WorkflowEdge, WorkflowNode, WorkflowRunReport } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { EmptyCanvas } from "./EmptyCanvas";
import { connectionOptions } from "./forms/DataForms";
import { Inspector } from "./Inspector";
import { autoLayout, canConnect, edgeId, leavesOf, newNode, removeNodes, upstreamOf, type NodeKind, type Wire } from "./model";
import { NodePicker } from "./NodePicker";
import { pickOutputFile } from "./pickers";
import { EMPTY_VIEW, ensureRunListener, startRun, stopRun, useRunStore } from "./runStore";
import { WorkflowCanvas } from "./WorkflowCanvas";

/// Run all pauses this long between levels so the data can be watched moving.
const WATCH_DELAY_MS = 350;
const AUTOSAVE_MS = 1500;

interface EditorState {
  name: string;
  connectionId: string;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  schedulesActive: boolean;
  dirty: boolean;
  /// Counts edits, so a save that finishes after a newer edit leaves the document dirty.
  version: number;
}

function stateOf(doc: Document): EditorState {
  const body: WorkflowBody = doc.body.kind === "workflow" ? doc.body.data : {};
  const nodes = body.nodes ?? [];
  const edges = body.edges ?? [];
  // A workflow written without positions (by the AI assistant) arrives with every node at 0, 0.
  const unplaced = nodes.length > 1 && nodes.every((n) => n.x === 0 && n.y === 0);
  return {
    name: doc.name,
    connectionId: doc.connectionId ?? "",
    nodes: unplaced ? autoLayout(nodes, edges) : nodes,
    edges,
    schedulesActive: body.schedulesActive ?? false,
    dirty: false,
    version: 0,
  };
}

// WHAT:  A saved node's secrets, copied onto the editor's copy after a save.
// WHY:   What was typed (`plain`) has been sealed by the core; the editor must stop
//        holding it and show "saved" from then on.
function adoptSecrets(local: WorkflowNode[], saved: readonly WorkflowNode[]): WorkflowNode[] {
  return local.map((node) => {
    const stored = saved.find((s) => s.id === node.id);
    if (stored === undefined) return node;
    if (node.config.kind === "email" && stored.config.kind === "email") return { ...node, config: { ...node.config, password: stored.config.password } };
    if (node.config.kind === "slack" && stored.config.kind === "slack") return { ...node, config: { ...node.config, webhook: stored.config.webhook } };
    return node;
  });
}

function summaryOf(report: WorkflowRunReport): { ok: boolean; text: string } {
  const failed = report.nodes.filter((n) => n.state === "error").length;
  const ran = report.nodes.filter((n) => n.state === "success").length;
  if (report.status === "stopped") return { ok: true, text: `Stopped after ${ran} node${ran === 1 ? "" : "s"}` };
  if (report.status === "failed") return { ok: false, text: failed === 1 ? "Finished with 1 failed node" : `Finished with ${failed} failed nodes` };
  return { ok: true, text: `Completed: ${ran} node${ran === 1 ? "" : "s"} ran` };
}

export function WorkflowTab({ document: doc }: { document: Document }) {
  return (
    <ReactFlowProvider>
      <WorkflowEditor doc={doc} />
    </ReactFlowProvider>
  );
}

// WHAT:  The workflow editor: a canvas of nodes and wires, an inspector for the
//        selected node, and a toolbar to run, stop, save and schedule.
// WHY:   A workflow is a graph that runs in the core, node by node; this tab edits
//        the saved document and shows the run as the core reports it.
// HOW:   Edits autosave after a short pause and always before a run, because the core
//        runs what is stored (including each node's saved destructive opt-in). A save
//        by someone else (the AI assistant) replaces the view unless there are
//        unsaved edits here, which win. Run all pauses briefly between levels so the
//        data can be watched.
// WHERE: src-tauri/src/services/workflows (engine), src/features/workflows/runStore.ts
function WorkflowEditor({ doc }: { doc: Document }) {
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const connections = useWorkspace((s) => s.connections);
  const activeConnectionId = useWorkspace((s) => s.activeConnectionId);
  const openChat = useWorkspace((s) => s.openChat);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const { fitView } = useReactFlow();
  const view = useRunStore((s) => s.views[doc.id] ?? EMPTY_VIEW);

  const [state, setState] = useState<EditorState>(() => stateOf(doc));
  const latest = useRef(state);
  const docRef = useRef(doc);
  const stamp = useRef(doc.updatedAt);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const saveRef = useRef<() => Promise<boolean>>(() => Promise.resolve(true));

  useEffect(() => {
    ensureRunListener();
  }, []);
  useEffect(() => {
    docRef.current = doc;
  }, [doc]);

  const update = useCallback((change: (current: EditorState) => EditorState, markDirty = true) => {
    const next = change(latest.current);
    if (next === latest.current) return;
    const stamped = markDirty ? { ...next, dirty: true, version: next.version + 1 } : next;
    latest.current = stamped;
    setState(stamped);
  }, []);

  // ------------------------------------------------------------------- saving
  const save = useCallback(async (): Promise<boolean> => {
    const current = latest.current;
    const base = docRef.current;
    setSaving(true);
    try {
      const saved = await saveDocument({
        ...base,
        name: current.name.trim() === "" ? base.name : current.name.trim(),
        connectionId: current.connectionId === "" ? null : current.connectionId,
        body: {
          kind: "workflow",
          data: { nodes: current.nodes, edges: current.edges, schedulesActive: current.schedulesActive },
        },
      });
      stamp.current = saved.updatedAt;
      docRef.current = saved;
      if (latest.current.version === current.version) {
        const savedNodes = saved.body.kind === "workflow" ? (saved.body.data.nodes ?? []) : [];
        const next = { ...latest.current, nodes: adoptSecrets(latest.current.nodes, savedNodes), dirty: false };
        latest.current = next;
        setState(next);
      }
      return true;
    } catch (raw) {
      showError(normalizeError(raw));
      return false;
    } finally {
      setSaving(false);
    }
  }, [saveDocument, showError]);
  useEffect(() => {
    saveRef.current = save;
  }, [save]);

  useEffect(() => {
    if (!state.dirty) return;
    const timer = window.setTimeout(() => { void saveRef.current(); }, AUTOSAVE_MS);
    return () => { window.clearTimeout(timer); };
  }, [state.dirty, state.version]);

  // Someone else saved this workflow: show it unless edits here are still waiting to be saved.
  useEffect(() => {
    if (doc.updatedAt === stamp.current) return;
    stamp.current = doc.updatedAt;
    if (latest.current.dirty) return;
    const incoming = stateOf(doc);
    latest.current = incoming;
    setState(incoming);
    setSelectedId((id) => (incoming.nodes.some((n) => n.id === id) ? id : null));
  }, [doc]);

  // ------------------------------------------------------------------ editing
  const patchNode = useCallback(
    (next: WorkflowNode) => {
      update((s) => ({ ...s, nodes: s.nodes.map((n) => (n.id === next.id ? next : n)) }));
    },
    [update],
  );

  const addNode = useCallback(
    async (kind: NodeKind) => {
      let remembered: NodeConfig | null = null;
      if (kind === "email" || kind === "slack") {
        remembered = await ipc("workflow_last_node_config", { kind }).catch((): NodeConfig | null => null);
      }
      const created = newNode(kind, latest.current.nodes);
      const node = remembered !== null && remembered.kind === kind ? { ...created, config: remembered } : created;
      update((s) => ({ ...s, nodes: [...s.nodes, node] }));
      setSelectedId(node.id);
      window.setTimeout(() => { void fitView({ nodes: [{ id: node.id }], duration: 300, maxZoom: 1, padding: 1.2 }); }, 80);
    },
    [update, fitView],
  );

  const connect = useCallback(
    (wire: Wire) => {
      const verdict = canConnect(latest.current.nodes, latest.current.edges, wire);
      if (!verdict.ok) {
        showError(verdict.reason);
        return;
      }
      update((s) => ({ ...s, edges: [...s.edges, { id: edgeId(wire.source, wire.sourceHandle, wire.target), source: wire.source, target: wire.target, sourceHandle: wire.sourceHandle }] }));
    },
    [update, showError],
  );

  const deleteItems = useCallback(
    (nodeIds: readonly string[], edgeIds: readonly string[]) => {
      update((s) => {
        const removedEdges = new Set(edgeIds);
        const pruned = removeNodes(s.nodes, s.edges, new Set(nodeIds));
        const edges = pruned.edges.filter((e) => !removedEdges.has(e.id));
        if (pruned.nodes.length === s.nodes.length && edges.length === s.edges.length) return s;
        return { ...s, nodes: pruned.nodes, edges };
      });
      setSelectedId((id) => (id !== null && nodeIds.includes(id) ? null : id));
    },
    [update],
  );

  const moveNodes = useCallback(
    (moves: readonly { id: string; x: number; y: number }[]) => {
      update((s) => ({
        ...s,
        nodes: s.nodes.map((n) => {
          const move = moves.find((m) => m.id === n.id);
          return move === undefined || (move.x === n.x && move.y === n.y) ? n : { ...n, x: move.x, y: move.y };
        }),
      }));
    },
    [update],
  );

  const setNoteText = useCallback(
    (id: string, text: string) => {
      update((s) => ({ ...s, nodes: s.nodes.map((n) => (n.id === id && n.config.kind === "note" ? { ...n, config: { ...n.config, text } } : n)) }));
    },
    [update],
  );

  const tidy = () => {
    update((s) => ({ ...s, nodes: autoLayout(s.nodes, s.edges) }));
    window.setTimeout(() => { void fitView({ duration: 300, padding: 0.25, maxZoom: 1 }); }, 80);
  };

  // --------------------------------------------------------------------- running
  // WHAT:  Saves, asks for any Save-to-file path that is still blank, and runs.
  // WHY:   The first run of a Save node is where its path is chosen, once; a
  //        scheduled run has nobody to ask, so a blank path would only fail there.
  const run = useCallback(
    async (nodeId?: string) => {
      const current = latest.current;
      const involved = nodeId === undefined ? new Set(current.nodes.map((n) => n.id)) : upstreamOf(current.edges, nodeId);
      for (const node of current.nodes) {
        if (!involved.has(node.id) || node.config.kind !== "save_file" || node.config.path.trim() !== "") continue;
        const path = await pickOutputFile(current.name, node.config.format);
        if (path === null) return;
        const config = node.config;
        update((s) => ({ ...s, nodes: s.nodes.map((n) => (n.id === node.id ? { ...n, config: { ...config, path } } : n)) }));
      }
      if (!(await saveRef.current())) return;
      try {
        const report = await startRun(doc.id, { ...(nodeId !== undefined ? { nodeId } : { stepDelayMs: WATCH_DELAY_MS }) });
        const summary = summaryOf(report);
        if (summary.ok) showInfo(summary.text);
        else showError(summary.text);
      } catch (raw) {
        showError(normalizeError(raw));
      }
    },
    [doc.id, showError, showInfo, update],
  );

  const stop = useCallback(() => {
    void (async () => {
      try {
        await stopRun(doc.id);
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  }, [doc.id, showError]);

  const toggleSchedules = () => {
    update((s) => ({ ...s, schedulesActive: !s.schedulesActive }));
    window.setTimeout(() => { void saveRef.current(); }, 0);
  };

  // ------------------------------------------------------------------- the view
  const selected = state.nodes.find((n) => n.id === selectedId) ?? null;
  const hasSchedule = state.nodes.some((n) => n.config.kind === "schedule");
  const runnable = leavesOf(state.nodes, state.edges).length > 0;
  const askConnection = state.connectionId !== "" ? state.connectionId : activeConnectionId;

  const inputColumns = useMemo(() => {
    if (selected === null) return [];
    const names: string[] = [];
    for (const edge of state.edges.filter((e) => e.target === selected.id)) {
      const previews = view.outputs[edge.source] ?? [];
      const preview = previews.find((p) => p.handle === edge.sourceHandle) ?? previews[0];
      for (const column of preview?.columns ?? []) if (!names.includes(column.name)) names.push(column.name);
    }
    return names;
  }, [selected, state.edges, view.outputs]);

  const saveStatus = saving ? "Saving…" : state.dirty ? "Unsaved changes" : "Saved";
  const nodeCount = state.nodes.length;

  return (
    <div
      className="flex h-full min-h-0"
      onKeyDown={(event) => {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
          event.preventDefault();
          void saveRef.current();
        }
      }}
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex app-toolbar shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface">
          <Input
            value={state.name}
            onChange={(event) => { update((s) => ({ ...s, name: event.target.value })); }}
            className="h-6 w-48 rounded-md bg-transparent text-xs text-foreground"
            aria-label="Workflow name"
          />
          <AppSelect
            ariaLabel="Default connection"
            value={state.connectionId}
            options={connectionOptions(connections, "").map((o) => (o.value === "" ? { value: "", label: "Pick a connection" } : o))}
            onChange={(connectionId) => { update((s) => ({ ...s, connectionId })); }}
            size="sm"
            className="w-44"
            icon="database"
          />
          <NodePicker onPick={(kind) => void addNode(kind)} />
          {view.running ? (
            <Button size="sm" variant="danger-soft" onClick={stop}>
              <Icon name="stop" size={12} />
              Stop
            </Button>
          ) : (
            <Button size="sm" disabled={!runnable} onClick={() => void run()}>
              <Icon name="play" size={12} />
              Run all
            </Button>
          )}
          {view.running ? <Badge variant="soft">Running…</Badge> : null}
          <div className="ml-auto flex items-center gap-1.5">
            {hasSchedule ? (
              <Button
                size="sm"
                variant={state.schedulesActive ? "soft" : "secondary"}
                aria-pressed={state.schedulesActive}
                title={state.schedulesActive ? "Scheduled fires run, even with this tab closed. Click to pause." : "Scheduled fires are ignored. Click to activate."}
                onClick={toggleSchedules}
              >
                <span className={cn("size-1.5 rounded-full", state.schedulesActive ? "bg-success" : "bg-muted/60")} />
                Schedules {state.schedulesActive ? "active" : "paused"}
              </Button>
            ) : null}
            <IconButton icon="layers" label="Tidy up layout" disabled={nodeCount < 2} onClick={tidy} />
            <span className="px-1 text-[11px] text-muted" aria-live="polite">
              {saveStatus}
            </span>
            <Button size="sm" variant="secondary" disabled={!state.dirty || saving} onClick={() => void saveRef.current()}>
              Save
            </Button>
          </div>
        </div>
        <div className="relative min-h-0 flex-1">
          <WorkflowCanvas
            nodes={state.nodes}
            edges={state.edges}
            statuses={view.nodes}
            running={view.running}
            tick={view.tick}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onMove={moveNodes}
            onConnect={connect}
            onDelete={deleteItems}
            onNoteText={setNoteText}
            onRunNode={(id) => void run(id)}
          />
          {nodeCount === 0 ? (
            <EmptyCanvas
              onPick={(kind) => void addNode(kind)}
              onAskAi={askConnection === null ? null : (prompt) => { openChat(askConnection, `Build the workflow "${state.name}" (document id ${doc.id}). ${prompt}`); }}
            />
          ) : null}
        </div>
      </div>
      {selected !== null ? (
        <Inspector
          key={selected.id}
          node={selected}
          workflowConnectionId={state.connectionId}
          columns={inputColumns}
          update={view.nodes[selected.id] ?? null}
          previews={view.outputs[selected.id] ?? []}
          running={view.running}
          onChange={patchNode}
          onRun={() => void run(selected.id)}
          onStop={stop}
          onDelete={() => { deleteItems([selected.id], []); }}
        />
      ) : null}
    </div>
  );
}
