// SOT: workflow-node-catalog, workflow-connection-rules, workflow-graph-edit, workflow-node-defaults, workflow-node-summary, workflow-auto-layout
import type { ConnectionColor, NodeConfig, SourceHandle, WorkflowEdge, WorkflowNode } from "@/lib/bindings";
import type { IconName } from "@/lib/icons";

// WHAT:  The vocabulary of the canvas: what node kinds exist, how they look and
//        connect, what a fresh one contains, and the graph edits that keep the
//        document valid.
// WHY:   Everything the canvas, the picker, the inspector and the AI import need to
//        agree on lives in one pure module, so a node kind is added in one place.
// WHERE: src-tauri/src/model/workflow.rs (the shapes), src/features/workflows/*

export type NodeKind = NodeConfig["kind"];
export type ConfigOf<K extends NodeKind> = Extract<NodeConfig, { kind: K }>;

export type NodeCategory = "triggers" | "data" | "transform" | "control" | "output" | "notify" | "annotation";

export const NODE_CATEGORIES: readonly { id: NodeCategory; label: string }[] = [
  { id: "triggers", label: "Triggers & scheduling" },
  { id: "data", label: "Data sources" },
  { id: "transform", label: "Transformation" },
  { id: "control", label: "Control flow" },
  { id: "output", label: "Output" },
  { id: "notify", label: "Notifications" },
  { id: "annotation", label: "Annotations" },
];

/// Which output a node's edges leave from: its only one, or a Branch's two sides.
export type OutputHandle = "default" | "true" | "false";

export interface NodeSpec {
  label: string;
  description: string;
  category: NodeCategory;
  icon: IconName;
  /// Palette key, drawn through the `--color-conn-*` tokens.
  color: ConnectionColor;
  /// Has an input handle (something can feed it).
  input: boolean;
  /// Output handles, top to bottom. Empty = a sink.
  outputs: readonly OutputHandle[];
  /// Extra words the picker's search matches.
  keywords: string;
}

export const NODE_SPECS: Record<NodeKind, NodeSpec> = {
  schedule: { label: "Schedule", description: "Run this workflow automatically on an interval or at a set time.", category: "triggers", icon: "clock", color: "pink", input: false, outputs: ["default"], keywords: "trigger cron timer interval daily" },
  query: { label: "Query", description: "Run a SQL statement and pass its rows on.", category: "data", icon: "database", color: "blue", input: true, outputs: ["default"], keywords: "sql select database" },
  http: { label: "HTTP request", description: "Call a REST endpoint and turn the JSON answer into rows.", category: "data", icon: "globe", color: "blue", input: true, outputs: ["default"], keywords: "api rest fetch get post json web" },
  read_file: { label: "Read file", description: "Load rows from a CSV, TSV, JSON or NDJSON file.", category: "data", icon: "file", color: "blue", input: false, outputs: ["default"], keywords: "csv tsv json ndjson import disk" },
  filter: { label: "Filter rows", description: "Keep only the rows that match a set of rules.", category: "control", icon: "filter", color: "amber", input: true, outputs: ["default"], keywords: "where rule condition" },
  row_count_gate: { label: "Row count gate", description: "Let rows through only while the row count satisfies a rule; otherwise stop the branch.", category: "control", icon: "shield", color: "amber", input: true, outputs: ["default"], keywords: "stop empty count threshold guard" },
  branch: { label: "Branch (true / false)", description: "Send matching rows out one side and the rest out the other.", category: "control", icon: "git-branch", color: "amber", input: true, outputs: ["true", "false"], keywords: "if else split router condition" },
  repeat: { label: "Repeat", description: "Run everything after it N times; each pass carries i and total.", category: "control", icon: "repeat", color: "amber", input: false, outputs: ["default"], keywords: "loop iterate pages count" },
  delay: { label: "Delay", description: "Wait before passing the rows on.", category: "control", icon: "timer", color: "amber", input: true, outputs: ["default"], keywords: "wait sleep pause rate limit" },
  regex: { label: "Regex", description: "Extract or replace text in one column of every row.", category: "transform", icon: "search", color: "violet", input: true, outputs: ["default"], keywords: "pattern match replace extract text" },
  javascript: { label: "JavaScript", description: "Transform the rows with your own code, in a sandbox.", category: "transform", icon: "braces", color: "violet", input: true, outputs: ["default"], keywords: "script code map reduce custom" },
  save_file: { label: "Save to file", description: "Write the rows to CSV, TSV, JSON, NDJSON, Markdown or SQL.", category: "output", icon: "download", color: "green", input: true, outputs: [], keywords: "export csv json markdown sql write disk" },
  insert: { label: "Insert into table", description: "Insert the rows into an existing table, one at a time.", category: "output", icon: "table", color: "green", input: true, outputs: [], keywords: "write load rows database" },
  notification: { label: "Desktop notification", description: "Show a native notification with a templated title and body.", category: "notify", icon: "bell", color: "teal", input: true, outputs: [], keywords: "alert popup toast" },
  email: { label: "Send email", description: "Send an email through your SMTP server.", category: "notify", icon: "send", color: "teal", input: true, outputs: [], keywords: "smtp mail message report" },
  slack: { label: "Send Slack message", description: "Post a message to a Slack incoming webhook.", category: "notify", icon: "message", color: "teal", input: true, outputs: [], keywords: "webhook chat channel" },
  note: { label: "Note", description: "A comment on the canvas. It is never run.", category: "annotation", icon: "note", color: "gray", input: false, outputs: [], keywords: "comment annotation label text" },
};

export const NODE_KINDS = Object.keys(NODE_SPECS).filter((k): k is NodeKind => k in NODE_SPECS);

export function specOf(kind: NodeKind): NodeSpec {
  return NODE_SPECS[kind];
}

export function titleOf(node: WorkflowNode): string {
  return node.label.trim() !== "" ? node.label : NODE_SPECS[node.config.kind].label;
}

// ---------------------------------------------------------------- defaults
/// The IANA zone of this machine, for a new Schedule node ("" when the browser will not say).
export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return "";
  }
}

const noSecret = { hasSecret: false, plain: null, clear: false, inheritLast: false } as const;

// WHAT:  A new node's config: every field present, with sensible starting values.
export function defaultConfig(kind: NodeKind): NodeConfig {
  switch (kind) {
    case "schedule":
      return { kind, mode: "interval", intervalAmount: 5, intervalUnit: "minutes", time: "09:00", days: ["mon", "tue", "wed", "thu", "fri"], timezone: localTimeZone() };
    case "query":
      return { kind, connectionId: null, sql: "", allowDestructive: false };
    case "http":
      return { kind, method: "get", url: "", headers: "", body: "", responsePath: "", allowPrivateNetwork: false };
    case "read_file":
      return { kind, path: "", format: "csv" };
    case "save_file":
      return { kind, path: "", format: "csv", tableName: "" };
    case "insert":
      return { kind, connectionId: null, schema: "", table: "" };
    case "filter":
      return { kind, group: { combinator: "and", rules: [] } };
    case "row_count_gate":
      return { kind, op: "gt", value: 0 };
    case "branch":
      return { kind, group: { combinator: "and", rules: [] } };
    case "repeat":
      return { kind, count: 3 };
    case "delay":
      return { kind, amount: 1, unit: "s" };
    case "regex":
      return { kind, sourceField: "", pattern: "", flags: "", mode: "extract", replacement: "", outputField: "" };
    case "javascript":
      return { kind, code: "return rows;" };
    case "notification":
      return { kind, title: "", body: "" };
    case "email":
      return { kind, host: "", port: 587, security: "auto", username: "", password: { ...noSecret }, from: "", to: "", subject: "", body: "" };
    case "slack":
      return { kind, webhook: { ...noSecret }, text: "" };
    case "note":
      return { kind, text: "" };
  }
}

// ---------------------------------------------------------------- ids and positions
function suffix(): string {
  return Math.random().toString(36).slice(2, 8);
}

export function newNodeId(kind: NodeKind, existing: readonly WorkflowNode[]): string {
  const taken = new Set(existing.map((n) => n.id));
  const base = kind.replace(/_/g, "-");
  for (;;) {
    const id = `${base}-${suffix()}`;
    if (!taken.has(id)) return id;
  }
}

export function edgeId(source: string, handle: SourceHandle | null, target: string): string {
  return `e-${source}-${handle ?? "out"}-${target}`;
}

export const NODE_WIDTH = 264;
export const NODE_HEIGHT = 92;
const GAP = 72;

// WHAT:  A free spot for a new node: to the right of the graph, clear of the others.
export function freePosition(nodes: readonly WorkflowNode[]): { x: number; y: number } {
  if (nodes.length === 0) return { x: 80, y: 120 };
  const right = Math.max(...nodes.map((n) => n.x));
  const averageY = nodes.reduce((sum, n) => sum + n.y, 0) / nodes.length;
  const x = right + NODE_WIDTH + GAP;
  let y = Math.round(averageY);
  const crowded = (cy: number) => nodes.some((n) => Math.abs(n.x - x) < NODE_WIDTH && Math.abs(n.y - cy) < NODE_HEIGHT + 24);
  for (let guard = 0; guard < 24 && crowded(y); guard += 1) y += NODE_HEIGHT + 32;
  return { x, y };
}

export function newNode(kind: NodeKind, existing: readonly WorkflowNode[], at?: { x: number; y: number }): WorkflowNode {
  const position = at ?? freePosition(existing);
  return { id: newNodeId(kind, existing), x: position.x, y: position.y, label: "", continueOnError: false, config: defaultConfig(kind) };
}

// ---------------------------------------------------------------- connecting
export interface Wire {
  source: string;
  target: string;
  sourceHandle: SourceHandle | null;
}

export type Verdict = { ok: true } | { ok: false; reason: string };

function reaches(edges: readonly WorkflowEdge[], from: string, to: string): boolean {
  const seen = new Set<string>();
  const stack = [from];
  while (stack.length > 0) {
    const id = stack.pop();
    if (id === undefined || seen.has(id)) continue;
    if (id === to) return true;
    seen.add(id);
    for (const edge of edges) if (edge.source === id) stack.push(edge.target);
  }
  return false;
}

// WHAT:  Whether a wire between two nodes is allowed, and if not, why.
// HOW:   A node accepts input only if its kind has an input; only a Branch has two
//        outputs and an edge from it must say which; no wire twice; no cycle.
export function canConnect(nodes: readonly WorkflowNode[], edges: readonly WorkflowEdge[], wire: Wire): Verdict {
  const source = nodes.find((n) => n.id === wire.source);
  const target = nodes.find((n) => n.id === wire.target);
  if (!source || !target) return { ok: false, reason: "That node no longer exists." };
  if (source.id === target.id) return { ok: false, reason: "A node cannot feed itself." };
  const from = NODE_SPECS[source.config.kind];
  const to = NODE_SPECS[target.config.kind];
  if (from.outputs.length === 0) return { ok: false, reason: `${from.label} has no output.` };
  if (!to.input) return { ok: false, reason: `${to.label} takes no input.` };
  const branching = source.config.kind === "branch";
  if (branching && wire.sourceHandle === null) return { ok: false, reason: "Connect from the True or the False side." };
  if (!branching && wire.sourceHandle !== null) return { ok: false, reason: `${from.label} has a single output.` };
  if (edges.some((e) => e.source === wire.source && e.target === wire.target && e.sourceHandle === wire.sourceHandle)) {
    return { ok: false, reason: "These nodes are already connected." };
  }
  if (reaches(edges, wire.target, wire.source)) return { ok: false, reason: "That would make the workflow loop back on itself." };
  return { ok: true };
}

// ---------------------------------------------------------------- graph edits
export function removeNodes(nodes: readonly WorkflowNode[], edges: readonly WorkflowEdge[], ids: ReadonlySet<string>): { nodes: WorkflowNode[]; edges: WorkflowEdge[] } {
  return {
    nodes: nodes.filter((n) => !ids.has(n.id)),
    edges: edges.filter((e) => !ids.has(e.source) && !ids.has(e.target)),
  };
}

/// Nodes a run of `target` would execute: the node and everything upstream.
export function upstreamOf(edges: readonly WorkflowEdge[], target: string): Set<string> {
  const seen = new Set<string>();
  const stack = [target];
  while (stack.length > 0) {
    const id = stack.pop();
    if (id === undefined || seen.has(id)) continue;
    seen.add(id);
    for (const edge of edges) if (edge.target === id) stack.push(edge.source);
  }
  return seen;
}

/// Nodes nothing is wired out of, notes excepted: what "Run all" runs.
export function leavesOf(nodes: readonly WorkflowNode[], edges: readonly WorkflowEdge[]): string[] {
  const sources = new Set(edges.map((e) => e.source));
  return nodes.filter((n) => n.config.kind !== "note" && !sources.has(n.id)).map((n) => n.id);
}

// ---------------------------------------------------------------- layout
// WHAT:  Lays the graph out left to right by depth, for a workflow that arrives
//        without positions (an import) or tangled by hand ("Tidy up").
export function autoLayout(nodes: readonly WorkflowNode[], edges: readonly WorkflowEdge[]): WorkflowNode[] {
  const depth = new Map<string, number>();
  const visit = (id: string, trail: ReadonlySet<string>): number => {
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (trail.has(id)) return 0;
    const next = new Set(trail).add(id);
    const parents = edges.filter((e) => e.target === id).map((e) => e.source);
    const level = parents.length === 0 ? 0 : 1 + Math.max(...parents.map((p) => visit(p, next)));
    depth.set(id, level);
    return level;
  };
  for (const node of nodes) visit(node.id, new Set());
  const rows = new Map<number, number>();
  return nodes.map((node) => {
    const level = depth.get(node.id) ?? 0;
    const row = rows.get(level) ?? 0;
    rows.set(level, row + 1);
    return { ...node, x: 80 + level * (NODE_WIDTH + GAP), y: 80 + row * (NODE_HEIGHT + 56) };
  });
}

// ---------------------------------------------------------------- summaries
function firstLine(text: string): string {
  return text.split("\n").map((l) => l.trim()).find((l) => l.length > 0) ?? "";
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

const DAY_ORDER = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
const DAY_LABEL = { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" } as const;

export function summarizeSchedule(config: ConfigOf<"schedule">): string {
  if (config.mode === "interval") {
    const unit = config.intervalUnit.replace(/s$/, "");
    if (config.intervalAmount < 1) return "Set an interval";
    return config.intervalAmount === 1 ? `Every ${unit}` : `Every ${config.intervalAmount} ${unit}s`;
  }
  const chosen = DAY_ORDER.filter((d) => config.days.includes(d));
  const days = chosen.length === 7 ? "Every day" : chosen.map((d) => DAY_LABEL[d]).join(", ");
  if (chosen.length === 0) return "Pick a weekday";
  return `${days} at ${config.time} (${config.timezone.trim() === "" ? "local time" : config.timezone.trim()})`;
}

const GATE_SYMBOLS = { gt: ">", gte: "≥", lt: "<", lte: "≤", eq: "=", neq: "≠" } as const;
const DELAY_UNITS = { ms: "ms", s: "s", m: "min" } as const;

// WHAT:  The one-line description a node card shows under its title.
export function summarize(config: NodeConfig): string {
  switch (config.kind) {
    case "schedule":
      return summarizeSchedule(config);
    case "query":
      return firstLine(config.sql) || "No SQL yet";
    case "http":
      return config.url.trim() === "" ? "No URL yet" : `${config.method.toUpperCase()} ${config.url.trim()}`;
    case "read_file":
      return config.path.trim() === "" ? "No file chosen" : baseName(config.path);
    case "save_file":
      return config.path.trim() === "" ? "Asks for a file on first run" : baseName(config.path);
    case "insert":
      return config.table.trim() === "" ? "No table set" : `→ ${config.schema.trim() === "" ? "" : `${config.schema.trim()}.`}${config.table.trim()}`;
    case "filter":
    case "branch":
      return config.group.rules.length === 0 ? "No rules" : `${config.group.rules.length} rule${config.group.rules.length === 1 ? "" : "s"}`;
    case "row_count_gate":
      return `row count ${GATE_SYMBOLS[config.op]} ${config.value}`;
    case "repeat":
      return config.count < 1 ? "Set a count" : `repeat ${config.count} time${config.count === 1 ? "" : "s"}`;
    case "delay":
      return `wait ${config.amount}${DELAY_UNITS[config.unit]}`;
    case "regex":
      return config.sourceField.trim() === "" || config.pattern.trim() === "" ? "No pattern yet" : `${config.sourceField.trim()} /${config.pattern.trim()}/`;
    case "javascript":
      return firstLine(config.code) || "No code yet";
    case "notification":
      return config.title.trim() || "No title yet";
    case "email":
      return config.to.trim() || config.subject.trim() || "No recipient yet";
    case "slack":
      return config.text.trim() || "No message yet";
    case "note":
      return config.text.trim() || "Empty note";
  }
}

// WHAT:  What is missing before the node can run, or null when it is ready.
// WHY:   The card shows a warning dot instead of letting a run fail on an empty field.
export function problemOf(node: WorkflowNode): string | null {
  const config = node.config;
  switch (config.kind) {
    case "query":
      return config.sql.trim() === "" ? "Write the SQL" : null;
    case "http":
      return config.url.trim() === "" ? "Set the URL" : null;
    case "read_file":
      return config.path.trim() === "" ? "Choose a file" : null;
    case "insert":
      return config.table.trim() === "" ? "Set the table" : null;
    case "regex":
      return config.sourceField.trim() === "" ? "Set the source field" : config.pattern.trim() === "" ? "Set the pattern" : null;
    case "notification":
      return config.title.trim() === "" ? "Set the title" : null;
    case "email":
      return config.host.trim() === "" ? "Set the SMTP host" : config.from.trim() === "" ? "Set the sender" : config.to.trim() === "" ? "Set the recipient" : null;
    case "slack":
      return !config.webhook.hasSecret && (config.webhook.plain ?? "").trim() === "" ? "Set the webhook URL" : config.text.trim() === "" ? "Set the message" : null;
    default:
      return null;
  }
}
