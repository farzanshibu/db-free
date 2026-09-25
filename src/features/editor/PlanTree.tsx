// SOT: plan-tree-view, visual-explain, plan-hotspots
import { useMemo, useState } from "react";
import type { PlanNode } from "@/lib/bindings";
import { Button } from "@/components/ui/button";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";

/// How hot one operator is, from its own share of the work.
type Heat = "danger" | "warning" | "normal";

interface Measured {
  node: PlanNode;
  /// Stable key: the path of child indexes from the root.
  path: string;
  depth: number;
  /// The subtree's figure (cost, or measured ms when the plan was analyzed).
  total: number | null;
  /// What this operator adds on top of its inputs.
  own: number | null;
}

// WHAT:  The figure a bar is drawn from. Measured time beats the estimate
//        when the plan carries it (EXPLAIN ANALYZE); otherwise the cost.
function figure(node: PlanNode, timed: boolean): number | null {
  return timed ? node.actualMs : node.cost;
}

function measure(root: PlanNode, timed: boolean): Measured[] {
  const out: Measured[] = [];
  const walk = (node: PlanNode, path: string, depth: number) => {
    const total = figure(node, timed);
    const inputs = node.children.reduce((sum, child) => sum + (figure(child, timed) ?? 0), 0);
    out.push({ node, path, depth, total, own: total === null ? null : Math.max(0, total - inputs) });
    node.children.forEach((child, i) => walk(child, `${path}.${i}`, depth + 1));
  };
  walk(root, "0", 0);
  return out;
}

// WHAT:  The operators that do most of the work: the single largest own share
//        is danger, the next two warning — as long as they matter (≥ 10% of
//        the whole), so a flat plan is not painted red for nothing.
function heats(rows: readonly Measured[]): Map<string, Heat> {
  const whole = rows[0]?.total ?? 0;
  const ranked = rows.filter((r) => r.own !== null && r.own > 0 && whole > 0 && r.own / whole >= 0.1).sort((a, b) => (b.own ?? 0) - (a.own ?? 0));
  const out = new Map<string, Heat>();
  ranked.slice(0, 3).forEach((r, i) => out.set(r.path, i === 0 ? "danger" : "warning"));
  return out;
}

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  return Number.isInteger(n) ? String(n) : n.toFixed(2);
}

const BAR: Record<Heat, string> = { danger: "bg-danger", warning: "bg-warning", normal: "bg-accent/60" };
const LABEL: Record<Heat, string> = { danger: "text-danger", warning: "text-warning", normal: "text-foreground" };

// WHAT:  An execution plan as a collapsible tree, one bar per operator.
// WHY:   The indented text makes the reader do the arithmetic; a bar sized by
//        cost and the hot operators in warning/danger show where the time goes.
// HOW:   Bars are relative to the root's figure (costs are in engine units, so
//        only proportions mean anything). Heat comes from each operator's own
//        share — its figure minus its inputs' — not its cumulative one, or the
//        root would always be the "most expensive".
// WHERE: src-tauri/src/integrations/plan.rs (the tree), QueryPane.tsx (host)
export function PlanTree({ root }: { root: PlanNode }) {
  const timed = root.actualMs !== null;
  const rows = useMemo(() => measure(root, timed), [root, timed]);
  const heat = useMemo(() => heats(rows), [rows]);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const whole = rows[0]?.total ?? null;
  const unit = timed ? "ms" : "cost";

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  // A row is hidden when any ancestor path is collapsed.
  const visible = rows.filter((r) => ![...collapsed].some((c) => r.path.startsWith(`${c}.`)));

  return (
    <div className="flex flex-col gap-0.5 text-xs" role="tree" aria-label="Execution plan">
      <div className="mb-1 flex items-center gap-3 px-1 text-[10.5px] text-muted">
        <span>Bars: {timed ? "measured time" : "estimated cost"} of each subtree, relative to the whole plan.</span>
        <span className="flex items-center gap-1"><span className="inline-block size-2 rounded-sm bg-danger" />most work</span>
        <span className="flex items-center gap-1"><span className="inline-block size-2 rounded-sm bg-warning" />heavy</span>
      </div>
      {visible.map((r) => {
        const h = heat.get(r.path) ?? "normal";
        const share = r.total !== null && whole !== null && whole > 0 ? Math.min(1, r.total / whole) : null;
        const hasChildren = r.node.children.length > 0;
        const open = !collapsed.has(r.path);
        return (
          <div key={r.path} role="treeitem" aria-expanded={hasChildren ? open : undefined} aria-level={r.depth + 1} className="rounded-md px-1 py-1 hover:bg-surface-secondary/60">
            <div className="flex items-start gap-2">
              <div className="flex min-w-0 flex-1 items-start gap-1" style={{ paddingLeft: r.depth * 16 }}>
                {hasChildren ? (
                  <Button variant="ghost" size="icon-sm" className="size-4 shrink-0" aria-label={open ? "Collapse" : "Expand"} onClick={() => toggle(r.path)}>
                    <Icon name={open ? "chevron-down" : "chevron-right"} size={11} />
                  </Button>
                ) : (
                  <span className="inline-block size-4 shrink-0" />
                )}
                <div className="min-w-0">
                  <div className={cn("selectable truncate font-mono text-[11.5px] font-medium", LABEL[h])} title={r.node.label}>{r.node.label}</div>
                  {r.node.detail !== null ? <div className="selectable whitespace-pre-wrap font-mono text-[10.5px] text-muted">{r.node.detail}</div> : null}
                </div>
              </div>
              <div className="flex w-56 shrink-0 flex-col items-end gap-1 pt-0.5">
                <div className="flex gap-3 font-mono text-[10.5px] text-muted">
                  {r.node.rows !== null ? <span title="Rows">{fmt(r.node.rows)} rows</span> : null}
                  {r.total !== null ? <span title={unit}>{fmt(r.total)} {unit}</span> : null}
                </div>
                {share !== null ? (
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-secondary">
                    <div className={cn("h-full rounded-full", BAR[h])} style={{ width: `${Math.max(2, share * 100)}%` }} />
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
