// SOT: connection-picker, engine-grid, engine-categories-ui, connection-string-detect
import { useState } from "react";
import type { EngineKind } from "@/lib/bindings";
import { CATEGORIES, COMING_SOON, ENGINE_ORDER, PRESETS, blankInput, engineMeta, enginesOfKind, parseConnectionStringDetailed } from "@/lib/engines";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input, SearchInput } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { AppSelect } from "@/components/global/Field";
import { WithTip } from "@/components/global/InfoTip";
import { AutoGrid, Page, PageHeader } from "@/components/global/Layout";

// WHAT:  First step of "New connection": paste a connection string (auto-detects
//        the engine) or pick an engine from the catalogue, grouped by category
//        (relational, document, key-value, graph, vector, …).
export function ConnectionPicker() {
  const openForm = useWorkspace((s) => s.openForm);
  const goConnections = useWorkspace((s) => s.goConnections);
  const [text, setText] = useState("");
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<EngineKind | "all">("all");
  const detected = parseConnectionStringDetailed(text);
  const parsed = detected?.input ?? null;
  const needle = search.trim().toLowerCase();
  const sections = CATEGORIES.filter((c) => category === "all" || c.kind === category).map((c) => ({
    ...c,
    engines: enginesOfKind(c.kind).filter((e) => {
      if (needle.length === 0) return true;
      const meta = engineMeta(e);
      return meta.label.toLowerCase().includes(needle) || meta.hint.toLowerCase().includes(needle) || c.label.toLowerCase().includes(needle);
    }),
  })).filter((s) => s.engines.length > 0);

  return (
    <Page
      header={
        <PageHeader
          title="New Connection"
          onBack={goConnections}
          tip="Paste a connection string to detect the engine, or pick one below."
          meta={`${ENGINE_ORDER.length} engines`}
        />
      }
    >
      <Card className="glass-card rounded-lg p-3 shadow-xs border-border/40">
        <CardContent className="p-0">
          <Input
            value={text}
            onChange={(event) => { setText(event.target.value); }}
            aria-label="Connection string"
            placeholder="Paste a connection string: protocol://user:password@host:port/database"
            className="w-full font-mono text-xs"
          />
          {parsed ? (
            <div className="mt-2.5 flex flex-col gap-2 border-t border-border/40 pt-2">
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-2 text-xs text-success font-medium">
                  <Icon name="check" size={13} />
                  Detected {engineMeta(parsed.engine).label}
                </span>
                <Button onClick={() => openForm(undefined, undefined, parsed)} className="font-semibold liquid-hover">
                  Continue
                  <Icon name="chevron-right" size={14} />
                </Button>
              </div>
              {detected !== null && detected.notes.length > 0 ? <p className="text-xs text-muted">{detected.notes.join(" ")}</p> : null}
            </div>
          ) : text.trim().length > 0 ? (
            <Alert variant="danger" className="mt-2 text-xs rounded-lg">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle>Unrecognised Scheme</AlertTitle>
                <AlertDescription>Supported: {ENGINE_ORDER.flatMap((e) => engineMeta(e).schemes).join(", ")}.</AlertDescription>
              </AlertContent>
            </Alert>
          ) : null}
        </CardContent>
      </Card>

      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={search} onChange={setSearch} aria-label="Search engines" placeholder="Search engines…" className="glass-input w-64 min-w-0 rounded-md text-xs" />
        <AppSelect<EngineKind | "all">
          ariaLabel="Category"
          value={category}
          options={[{ value: "all", label: "All categories" }, ...CATEGORIES.map((c) => ({ value: c.kind, label: c.label }))]}
          onChange={setCategory}
          className="w-52"
        />
      </div>

      {sections.length === 0 ? <p className="text-center text-xs text-muted">No engine matches “{search}”.</p> : null}

      {sections.map((section) => (
        <section key={section.kind} className="flex flex-col gap-2">
          <h2 className="px-0.5 text-xs font-semibold uppercase tracking-wider text-foreground/90">
            <WithTip tip={section.blurb} tipLabel={`About ${section.label}`}>{section.label}</WithTip>
          </h2>
          <AutoGrid min="xs">
            {section.engines.map((engine) => {
              const meta = engineMeta(engine);
              return (
                <Button
                  key={engine}
                  variant="ghost"
                  onClick={() => openForm(undefined, undefined, blankInput(engine))}
                  className="group flex h-auto w-full items-center justify-start gap-2 rounded-lg glass-card px-2 py-1.5 text-left glass-card-hover border border-border/40"
                >
                  <span className="flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-md border border-border/40 bg-surface-tertiary/70 shadow-xs group-hover:scale-105 transition-transform">
                    <EngineIcon engine={engine} size={18} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-xs font-semibold text-foreground tracking-tight">{meta.label}</span>
                    <span className="block truncate text-[11px] text-muted">{meta.hint}</span>
                  </span>
                </Button>
              );
            })}
          </AutoGrid>
        </section>
      ))}

      {PRESETS.length > 0 || COMING_SOON.length > 0 ? (
        <AutoGrid min="xs">
          {PRESETS.map((preset) => (
            <Button
              key={preset.id}
              variant="ghost"
              onClick={() => openForm(undefined, preset, blankInput(preset.engine, preset))}
              className="group flex h-auto w-full items-center justify-start gap-2 rounded-lg glass-card px-2 py-1.5 text-left glass-card-hover border border-border/40"
            >
              <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-surface-tertiary/70 text-success shadow-xs border border-border/40">
                <Icon name="database" size={14} />
              </span>
              <span className="min-w-0">
                <span className="block truncate text-xs font-semibold text-foreground tracking-tight">{preset.label}</span>
                <span className="block truncate text-[11px] text-muted">{preset.hint}</span>
              </span>
            </Button>
          ))}
          {COMING_SOON.map((item) => (
            <Tooltip key={item.label}>
              <TooltipTrigger asChild>
                <div tabIndex={0} className="flex items-center gap-2 rounded-lg border border-dashed border-border/40 bg-surface/20 px-2 py-1.5 opacity-45 outline-none focus-visible:ring-2 focus-visible:ring-ring/50" aria-disabled="true">
                  <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-surface-tertiary/40 text-muted">
                    <Icon name="database" size={14} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-xs text-foreground">{item.label}</span>
                    <span className="block truncate text-[11px] text-muted">coming soon</span>
                  </span>
                </div>
              </TooltipTrigger>
              <TooltipContent>{item.hint}</TooltipContent>
            </Tooltip>
          ))}
        </AutoGrid>
      ) : null}
    </Page>
  );
}
