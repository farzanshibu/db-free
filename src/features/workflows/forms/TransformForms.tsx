// SOT: workflow-regex-form, regex-live-test, workflow-javascript-form
import { useEffect, useId, useState } from "react";
import type { RegexMode, RegexTestResult } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { Field, Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { JS_SNIPPETS } from "../snippets";
import { ColumnList, type FormProps } from "./common";

const MODES: readonly { value: RegexMode; label: string }[] = [
  { value: "extract", label: "Extract" },
  { value: "replace", label: "Replace" },
];

const FLAGS = [
  { flag: "i", label: "Ignore case" },
  { flag: "m", label: "Multi-line" },
  { flag: "s", label: "Dot matches newline" },
  { flag: "g", label: "All matches" },
] as const;

// WHAT:  The Regex node: one column in, one column out, with a live tester.
// WHY:   The tester runs the same engine the workflow runs (the core's), so what
//        matches here is what matches in the run, look-around limits included.
export function RegexForm({ config, onChange, context }: FormProps<"regex">) {
  const listId = useId();
  const [sample, setSample] = useState("");
  const [result, setResult] = useState<RegexTestResult | null>(null);

  const testing = config.pattern !== "" && sample !== "";
  useEffect(() => {
    if (!testing) return;
    let live = true;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const next = await ipc("test_workflow_regex", { pattern: config.pattern, flags: config.flags, mode: config.mode, replacement: config.replacement, sample });
          if (live) setResult(next);
        } catch (raw) {
          if (live) setResult({ matched: false, output: null, error: normalizeError(raw).message });
        }
      })();
    }, 250);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [testing, config.pattern, config.flags, config.mode, config.replacement, sample]);

  const toggleFlag = (flag: string) => {
    const next = config.flags.includes(flag) ? config.flags.replace(flag, "") : `${config.flags}${flag}`;
    onChange({ ...config, flags: next });
  };

  return (
    <div className="flex flex-col gap-2">
      <ColumnList id={listId} columns={context.columns} />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${listId}-source`}>Source column</Label>
        <Input id={`${listId}-source`} list={listId} className="font-mono" value={config.sourceField} placeholder="email" onChange={(event) => { onChange({ ...config, sourceField: event.target.value }); }} />
      </div>
      <Segmented value={config.mode} options={MODES} onChange={(mode) => { onChange({ ...config, mode }); }} label="Mode" />
      <Field label="Pattern" mono value={config.pattern} placeholder={String.raw`(\w+)@example\.com`} onChange={(pattern) => { onChange({ ...config, pattern }); }} />
      <div className="flex flex-wrap gap-1" role="group" aria-label="Flags">
        {FLAGS.map(({ flag, label }) => (
          <Button key={flag} size="xs" variant={config.flags.includes(flag) ? "primary" : "secondary"} aria-pressed={config.flags.includes(flag)} title={label} onClick={() => { toggleFlag(flag); }}>
            {flag} <span className="font-normal opacity-80">{label}</span>
          </Button>
        ))}
      </div>
      {config.mode === "replace" ? (
        <Field label="Replacement" mono value={config.replacement} placeholder="$1" description="$1, $2 … are the capture groups, $& the whole match." onChange={(replacement) => { onChange({ ...config, replacement }); }} />
      ) : null}
      <Field
        label="Write the result to"
        optional
        mono
        value={config.outputField}
        placeholder={config.sourceField === "" ? "Same column" : config.sourceField}
        description="A new name adds a column and keeps the original."
        onChange={(outputField) => { onChange({ ...config, outputField }); }}
      />
      <div className="flex flex-col gap-1.5 rounded-lg border border-border bg-surface-secondary/40 p-2">
        <Label>Try it</Label>
        <Input value={sample} placeholder="A sample value from the column" className="font-mono text-xs" onChange={(event) => { setSample(event.target.value); }} />
        {!testing || result === null ? null : result.error !== null ? (
          <p className="flex items-start gap-1.5 text-xs text-danger">
            <Icon name="alert" size={12} className="mt-0.5 shrink-0" />
            {result.error}
          </p>
        ) : (
          <p className="break-all font-mono text-xs">
            <span className={result.matched ? "text-success" : "text-muted"}>{result.matched ? "matches" : "no match"}</span>
            {result.output !== null ? (
              <>
                {" → "}
                <span className="text-foreground">{result.output === "" ? "(empty)" : result.output}</span>
              </>
            ) : config.mode === "extract" ? (
              <span className="text-muted"> → NULL</span>
            ) : null}
          </p>
        )}
      </div>
      <p className="text-[11px] leading-relaxed text-muted">Rust regex syntax: no look-around and no back-references. Without the All matches flag, Replace changes the first match only.</p>
    </div>
  );
}

// WHAT:  The JavaScript node: the body of an async function over `rows`.
// WHY:   It runs in a sandbox with no network, no files and no process access; Stop
//        ends it even in a busy loop, and `console` output appears in the node's Output tab.
export function JavascriptForm({ config, onChange }: FormProps<"javascript">) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <Label>Code</Label>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="xs">
              <Icon name="sparkles" size={12} />
              Snippets
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72">
            {JS_SNIPPETS.map((snippet) => (
              <DropdownMenuItem key={snippet.label} onSelect={() => { onChange({ ...config, code: snippet.code }); }}>
                <span className="flex flex-col">
                  <span className="text-xs">{snippet.label}</span>
                  <span className="text-[11px] text-muted">{snippet.description}</span>
                </span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <Textarea
        value={config.code}
        rows={14}
        spellCheck={false}
        aria-label="Code"
        className="min-h-56 whitespace-pre text-xs leading-relaxed"
        onChange={(event) => { onChange({ ...config, code: event.target.value }); }}
        onKeyDown={(event) => {
          if (event.key !== "Tab") return;
          event.preventDefault();
          const target = event.currentTarget;
          const { selectionStart: start, selectionEnd: end } = target;
          onChange({ ...config, code: `${config.code.slice(0, start)}  ${config.code.slice(end)}` });
          window.requestAnimationFrame(() => { target.setSelectionRange(start + 2, start + 2); });
        }}
      />
      <p className="text-[11px] leading-relaxed text-muted">
        You get <code className="font-mono text-foreground">rows</code> (an array of objects), <code className="font-mono text-foreground">fields</code> (column names and types) and{" "}
        <code className="font-mono text-foreground">ctx.aborted</code> (true after Stop). Return an array to replace the rows, <code className="font-mono text-foreground">{"{ rows, fields }"}</code>{" "}
        to set the columns too, or nothing to pass the input through. <code className="font-mono text-foreground">await</code> and <code className="font-mono text-foreground">setTimeout</code> work; there is no{" "}
        <code className="font-mono text-foreground">fetch</code>, <code className="font-mono text-foreground">require</code> or file access.
      </p>
    </div>
  );
}
