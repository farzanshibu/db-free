// SOT: connection-picker, engine-grid, engine-categories-ui, connection-string-detect
import { useState } from "react";
import { CATEGORIES, COMING_SOON, ENGINE_ORDER, PRESETS, blankInput, engineMeta, enginesOfKind, parseConnectionString } from "@/lib/engines";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input, SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";

// WHAT:  First step of "New connection": paste a connection string (auto-detects
//        the engine) or pick an engine from the catalogue, grouped by category
//        (relational, document, key-value, graph, vector, …).
export function ConnectionPicker() {
  const openForm = useWorkspace((s) => s.openForm);
  const goConnections = useWorkspace((s) => s.goConnections);
  const [text, setText] = useState("");
  const [search, setSearch] = useState("");
  const parsed = parseConnectionString(text);
  const needle = search.trim().toLowerCase();
  const sections = CATEGORIES.map((c) => ({
    ...c,
    engines: enginesOfKind(c.kind).filter((e) => {
      if (needle.length === 0) return true;
      const meta = engineMeta(e);
      return meta.label.toLowerCase().includes(needle) || meta.hint.toLowerCase().includes(needle) || c.label.toLowerCase().includes(needle);
    }),
  })).filter((s) => s.engines.length > 0);

  return (
    <div className="grid-bg flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region flex h-11 app-pad-x shrink-0 items-center gap-2 border-b border-border/40 glass-header" data-tauri-drag-region>
        <Button variant="ghost" size="sm" onClick={goConnections} className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover">
          <Icon name="chevron-left" size={14} />
          Back
        </Button>
        <span className="text-sm font-semibold text-foreground tracking-tight" data-tauri-drag-region>
          New Connection
        </span>
        <div className="drag-region h-full flex-1" data-tauri-drag-region />
        <span className="text-[11px] text-muted">{ENGINE_ORDER.length} engines · {CATEGORIES.length} categories</span>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-6 pt-8 pb-12">
          <div className="text-center">
            <h1 className="text-xl font-bold tracking-tight text-foreground">Select a Database</h1>
            <p className="mt-1 text-xs text-muted">Paste your connection string or pick an engine to get started.</p>
          </div>

          <Card className="glass-card rounded-2xl p-5 shadow-lg border-border/40">
            <CardContent className="p-0">
              <Input
                value={text}
                onChange={(event) => { setText(event.target.value); }}
                placeholder="protocol://user:password@host:port/database"
                className="mt-1.5 w-full font-mono text-xs"
              />
              {parsed ? (
                <div className="mt-3.5 flex items-center justify-between border-t border-border/40 pt-3">
                  <span className="flex items-center gap-2 text-xs text-success font-medium">
                    <Icon name="check" size={13} />
                    Detected {engineMeta(parsed.engine).label}
                  </span>
                  <Button onClick={() => openForm(undefined, undefined, parsed)} className="font-semibold liquid-hover">
                    Continue
                    <Icon name="chevron-right" size={14} />
                  </Button>
                </div>
              ) : text.trim().length > 0 ? (
                <Alert variant="danger" className="mt-3 text-xs rounded-xl">
                  <AlertIndicator />
                  <AlertContent>
                    <AlertTitle>Unrecognised Scheme</AlertTitle>
                    <AlertDescription>Supported: {ENGINE_ORDER.flatMap((e) => engineMeta(e).schemes).join(", ")}.</AlertDescription>
                  </AlertContent>
                </Alert>
              ) : null}
            </CardContent>
          </Card>

          <div className="flex items-center gap-3 text-xs text-muted">
            <Separator className="flex-1 opacity-50" />
            <span className="text-[11px] uppercase tracking-wider font-semibold text-muted/70">or select engine</span>
            <Separator className="flex-1 opacity-50" />
          </div>

          <SearchInput value={search} onChange={setSearch} aria-label="Search engines" placeholder="Search engines or categories…" className="glass-input rounded-xl h-9 w-full text-xs" />

          {sections.length === 0 ? <p className="text-center text-xs text-muted">No engine matches “{search}”.</p> : null}

          {sections.map((section) => (
            <section key={section.kind} className="flex flex-col gap-2.5">
              <div className="flex items-baseline gap-2 px-0.5">
                <h2 className="text-xs font-semibold uppercase tracking-wider text-foreground/90">{section.label}</h2>
                <span className="text-[11px] text-muted">{section.blurb}</span>
              </div>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
                {section.engines.map((engine) => {
                  const meta = engineMeta(engine);
                  return (
                    <Button
                      key={engine}
                      variant="ghost"
                      onClick={() => openForm(undefined, undefined, blankInput(engine))}
                      className="group flex h-auto w-full items-center justify-start gap-3 rounded-xl glass-card px-3 py-2.5 text-left glass-card-hover border border-border/40"
                    >
                      <span className="flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-border/40 bg-surface-tertiary/70 shadow-xs group-hover:scale-105 transition-transform">
                        <EngineIcon engine={engine} size={24} />
                      </span>
                      <span className="min-w-0">
                        <span className="block truncate text-[13px] font-semibold text-foreground tracking-tight">{meta.label}</span>
                        <span className="block truncate text-[11px] text-muted">{meta.hint}</span>
                      </span>
                    </Button>
                  );
                })}
              </div>
            </section>
          ))}

          {PRESETS.length > 0 || COMING_SOON.length > 0 ? (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              {PRESETS.map((preset) => (
                <Button
                  key={preset.id}
                  variant="ghost"
                  onClick={() => openForm(undefined, preset, blankInput(preset.engine, preset))}
                  className="group flex h-auto w-full items-center justify-start gap-3 rounded-xl glass-card px-3 py-2.5 text-left glass-card-hover border border-border/40"
                >
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface-tertiary/70 text-success shadow-xs border border-border/40">
                    <Icon name="database" size={18} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] font-semibold text-foreground tracking-tight">{preset.label}</span>
                    <span className="block truncate text-[11px] text-muted">{preset.hint}</span>
                  </span>
                </Button>
              ))}
              {COMING_SOON.map((item) => (
                <div key={item.label} className="flex items-center gap-3 rounded-xl border border-dashed border-border/40 bg-surface/20 px-3 py-2.5 opacity-45" title={item.hint} aria-disabled="true">
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-surface-tertiary/40 text-muted">
                    <Icon name="database" size={18} />
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-[13px] text-foreground">{item.label}</span>
                    <span className="block truncate text-[11px] text-muted">coming soon</span>
                  </span>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      </ScrollArea>
    </div>
  );
}
