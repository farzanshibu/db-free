// SOT: capability-matrix, engine-feature-matrix, capabilities-page
import { useMemo, useState } from "react";
import type { Capabilities, Engine, EngineKind } from "@/lib/bindings";
import { CATEGORIES, ENGINE_ORDER, engineMeta } from "@/lib/engines";
import { OBJECT_KINDS, SECTIONS, TOOL_ORDER, kindMeta, profileOf, toolMeta } from "@/lib/objects";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { EngineIcon } from "@/components/global/EngineIcon";
import { AppSelect, Check } from "@/components/global/Field";
import { Page, PageHeader } from "@/components/global/Layout";
import { cn } from "@/lib/cn";
import { keysOf } from "@/lib/records";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

interface Row {
  id: string;
  label: string;
  section: string;
  has: (engine: Engine) => boolean;
}

const CAPABILITY_ROWS: readonly { key: keyof Capabilities; label: string }[] = [
  { key: "sql", label: "SQL in the query tab" },
  { key: "namespaces", label: "Schemas / namespaces" },
  { key: "fixedColumns", label: "Fixed columns per collection" },
  { key: "paging", label: "Server-side paging" },
  { key: "rowEstimate", label: "Row count estimate" },
  { key: "exactEstimate", label: "Exact counts" },
  { key: "views", label: "Views" },
  { key: "transactions", label: "Transactions" },
];

// WHAT:  Feature × engine matrix straight from every family's `profile()`:
//        base capabilities, playground tools, then every object kind by
//        section. The implementation roadmap and the "what can this app do
//        with my database" answer, on one page.
// WHERE: src/lib/objects.ts (profileOf), src-tauri/src/integrations/<family>.rs
export function CapabilityMatrixPage() {
  const goConnections = useWorkspace((s) => s.goConnections);
  const [category, setCategory] = useState<EngineKind | "all">("all");
  const [search, setSearch] = useState("");
  const [hideEmpty, setHideEmpty] = useState(true);

  const engines = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return ENGINE_ORDER.filter((e) => (category === "all" || engineMeta(e).kind === category) && (needle.length === 0 || engineMeta(e).label.toLowerCase().includes(needle) || e.includes(needle)));
  }, [category, search]);

  const rows = useMemo<Row[]>(() => {
    const out: Row[] = CAPABILITY_ROWS.map((c) => ({ id: `cap:${c.key}`, label: c.label, section: "Capabilities", has: (e) => profileOf(e).capabilities[c.key] }));
    for (const tool of TOOL_ORDER) {
      out.push({ id: `tool:${tool}`, label: toolMeta(tool).label, section: "Tools", has: (e) => profileOf(e).tools.includes(tool) });
    }
    for (const section of SECTIONS) {
      const kinds = keysOf(OBJECT_KINDS).filter((k) => kindMeta(k).section === section.id);
      for (const kind of kinds) {
        out.push({ id: `kind:${kind}`, label: kindMeta(kind).plural, section: section.label, has: (e) => profileOf(e).objectKinds.includes(kind) });
      }
    }
    return out;
  }, []);

  const visibleRows = hideEmpty ? rows.filter((r) => engines.some((e) => r.has(e))) : rows;
  const totals = useMemo(() => Object.fromEntries(engines.map((e) => [e, rows.filter((r) => r.has(e)).length])), [engines, rows]);
  const categoryOptions: { value: EngineKind | "all"; label: string }[] = [{ value: "all", label: "All categories" }, ...CATEGORIES.map((c) => ({ value: c.kind, label: c.label }))];

  return (
    <Page
      scroll={false}
      header={
        <PageHeader
          title="Engine capabilities"
          onBack={goConnections}
          backLabel="Connections"
          tip="Which features each engine's adapter supports: a check is yes, a dot is no. Hover an engine for its full name and feature count."
          meta={
            <Badge size="sm" variant="soft" className="font-mono text-[10px]">
              {engines.length} engines · {visibleRows.length} features
            </Badge>
          }
          actions={
            <>
              <AppSelect ariaLabel="Category" value={category} options={categoryOptions} size="sm" className="w-52" onChange={setCategory} />
              <div className="w-48 min-w-0">
                <SearchInput value={search} onChange={setSearch} aria-label="Search engines" placeholder="Engine…" className="w-full rounded-md text-xs glass-input" />
              </div>
              <label className="flex items-center gap-1.5 text-xs text-muted">
                <Check label="Hide empty rows" checked={hideEmpty} onChange={setHideEmpty} />
                Hide empty
              </label>
            </>
          }
        />
      }
    >
      <ScrollArea orientation="both" className="min-h-0 flex-1">
        <table className="min-w-max border-separate border-spacing-0 text-xs">
          <thead className="sticky top-0 z-20">
            <tr>
              <th className="sticky left-0 z-30 border-b border-r border-border/40 bg-background px-2.5 py-1.5 text-left font-medium text-muted">Feature</th>
              {engines.map((engine) => (
                <th key={engine} className="border-b border-border/40 bg-background px-1 py-1.5 align-bottom">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="flex flex-col items-center gap-1 rounded-sm px-1 py-1 outline-none focus-visible:ring-2 focus-visible:ring-ring/50" tabIndex={0}>
                        <EngineIcon engine={engine} size={16} />
                        <span className="w-14 truncate text-[9px] font-medium text-muted">{engineMeta(engine).label}</span>
                        <span className="font-mono text-[9px] text-muted/70">{totals[engine]}</span>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">
                      {engineMeta(engine).label} · {engineMeta(engine).hint} · {totals[engine]} features
                    </TooltipContent>
                  </Tooltip>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {visibleRows.map((row, i) => {
              const newSection = i === 0 || visibleRows[i - 1]?.section !== row.section;
              return (
                <RowGroup key={row.id} row={row} engines={engines} newSection={newSection} />
              );
            })}
          </tbody>
        </table>
      </ScrollArea>
    </Page>
  );
}

function RowGroup({ row, engines, newSection }: { row: Row; engines: readonly Engine[]; newSection: boolean }) {
  return (
    <>
      {newSection ? (
        <tr>
          <td colSpan={engines.length + 1} className="sticky left-0 border-b border-border/40 bg-surface px-2.5 pt-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted/80">
            {row.section}
          </td>
        </tr>
      ) : null}
      <tr className="group">
        <td className="sticky left-0 z-10 max-w-[240px] truncate border-b border-r border-separator bg-background px-2.5 py-1 text-foreground group-hover:bg-surface-secondary/60">{row.label}</td>
        {engines.map((engine) => {
          const has = row.has(engine);
          return (
            // One cell per engine × feature (thousands): a Tooltip each would be
            // heavy, so the answer is spelled out for screen readers and the
            // sticky header names the column.
            <td key={engine} className={cn("border-b border-separator px-1 py-1 text-center group-hover:bg-surface-secondary/40", has ? "text-success" : "text-muted/30")}>
              {has ? <Icon name="check" size={12} /> : <span aria-hidden="true">·</span>}
              <span className="sr-only">{`${engineMeta(engine).label}: ${has ? "yes" : "no"}`}</span>
            </td>
          );
        })}
      </tr>
    </>
  );
}

