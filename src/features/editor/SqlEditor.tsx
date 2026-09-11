// SOT: sql-editor, codemirror-setup, editor-theme, schema-completion, run-at-cursor, statement-gutter
import { useEffect, useRef } from "react";
import { EditorState, Compartment, StateEffect, StateField, type Extension } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  gutter,
  GutterMarker,
  Decoration,
  type DecorationSet,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { HighlightStyle, syntaxHighlighting, bracketMatching } from "@codemirror/language";
import { sql, MSSQL, MySQL, MariaSQL, PostgreSQL, SQLite, StandardSQL, type SQLDialect, type SQLNamespace } from "@codemirror/lang-sql";
import { tags } from "@lezer/highlight";
import type { Engine, StatementSpan } from "@/lib/bindings";

// WHAT:  The one statement a run will send, and where it came from.
// WHY:   PRD §4.3 — a tab holds a script; running the whole buffer when the caret
//        sits inside one statement of it is rarely what was meant.
// WHERE: src/features/editor/QueryPane.tsx (the Run button reads this)
export interface RunTarget {
  text: string;
  from: number;
  to: number;
  /// 1-based place in the script. 0 when the user highlighted the text themselves.
  index: number;
  total: number;
  selected: boolean;
}

interface SqlEditorProps {
  value: string;
  onChange: (value: string) => void;
  onRun: (target: RunTarget) => void;
  engine: Engine;
  schema: SQLNamespace;
  defaultSchema?: string | undefined;
  /// Statement boundaries from the Rust splitter — the block's own, not a second
  /// one. Left out (dashboard and workflow panes) the gutter stays away and a run
  /// sends the whole buffer, which is all those panes ever wanted.
  spans?: readonly StatementSpan[] | undefined;
  onRunAll?: (() => void) | undefined;
  onTargetChange?: ((target: RunTarget) => void) | undefined;
}

const NO_SPANS: readonly StatementSpan[] = [];

const setSpans = StateEffect.define<readonly StatementSpan[]>();

// WHAT:  The statement boundaries the gutter and Run at cursor read.
// WHY:   Rust answers a beat behind the typing. Between answers the offsets are
//        carried through each edit, or a character typed into the first statement
//        leaves every ▶ below it pointing one place too far left — and for the
//        ~150 ms until the next split lands, ⌘/Ctrl + Enter sends a statement cut
//        one character short.
// HOW:   The start associates backwards and the end forwards, so text typed at
//        either edge of a statement lands inside it rather than outside.
const spansField = StateField.define<readonly StatementSpan[]>({
  create: () => [],
  update(spans, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setSpans)) return effect.value;
    }
    if (!tr.docChanged || spans.length === 0) return spans;
    return spans.map((s) => ({ ...s, start: tr.changes.mapPos(s.start, -1), end: tr.changes.mapPos(s.end, 1) }));
  },
});

// WHAT:  Keeps a span inside the document.
// WHY:   Spans are computed a beat behind the typing that produced them; an offset
//        past the end of the doc makes `lineAt` throw rather than merely look wrong.
function clampSpan(state: EditorState, span: StatementSpan): { from: number; to: number } {
  const max = state.doc.length;
  const from = Math.min(Math.max(span.start, 0), max);
  return { from, to: Math.min(Math.max(span.end, from), max) };
}

// WHAT:  What ⌘/Ctrl + Enter, the toolbar Run and the gutter ▶ would send.
// HOW:   A highlighted range wins outright — the user said what to run. Otherwise
//        it is the statement the caret is inside, else the next one below it, else
//        the last, so a caret parked on the blank line under a script still runs
//        something sensible. Before the first split answers, it is the whole text.
function targetOf(state: EditorState): RunTarget {
  const spans = state.field(spansField, false) ?? [];
  const selection = state.selection.main;
  if (!selection.empty) {
    return {
      text: state.doc.sliceString(selection.from, selection.to),
      from: selection.from,
      to: selection.to,
      index: 0,
      total: spans.length,
      selected: true,
    };
  }
  const index = indexAt(state, spans, selection.head);
  const span = index === -1 ? undefined : spans[index];
  if (span === undefined) {
    return { text: state.doc.toString(), from: 0, to: state.doc.length, index: 0, total: spans.length, selected: false };
  }
  const { from, to } = clampSpan(state, span);
  return { text: state.doc.sliceString(from, to), from, to, index: index + 1, total: spans.length, selected: false };
}

function indexAt(state: EditorState, spans: readonly StatementSpan[], pos: number): number {
  const inside = spans.findIndex((s) => {
    const { from, to } = clampSpan(state, s);
    return pos >= from && pos <= to;
  });
  if (inside !== -1) return inside;
  const below = spans.findIndex((s) => clampSpan(state, s).from > pos);
  if (below !== -1) return below;
  return spans.length - 1;
}

/// The first line of a statement, which is where its ▶ sits.
function markerLine(state: EditorState, span: StatementSpan): number {
  return state.doc.lineAt(clampSpan(state, span).from).from;
}

function spanStartingAt(state: EditorState, lineFrom: number): number {
  return (state.field(spansField, false) ?? []).findIndex((s) => markerLine(state, s) === lineFrom);
}

const statementMark = Decoration.mark({ class: "cm-active-statement" });

// WHAT:  Tints the statement the caret is in, so what Run will send is visible
//        before it is sent. Only worth drawing when the buffer holds more than one.
function decorate(state: EditorState): DecorationSet {
  const target = targetOf(state);
  if (target.selected || target.index === 0 || target.total < 2 || target.to <= target.from) return Decoration.none;
  return Decoration.set([statementMark.range(target.from, target.to)]);
}

const activeStatement = StateField.define<DecorationSet>({
  create: (state) => decorate(state),
  update: (deco, tr) =>
    tr.docChanged || tr.selection || tr.effects.some((e) => e.is(setSpans)) ? decorate(tr.state) : deco,
  provide: (field) => EditorView.decorations.from(field),
});

// WHAT:  The ▶ in the gutter: one per statement, on the line it starts.
class RunMarker extends GutterMarker {
  constructor(
    private readonly active: boolean,
    private readonly danger: boolean,
  ) {
    super();
  }

  override eq(other: GutterMarker): boolean {
    return other instanceof RunMarker && other.active === this.active && other.danger === this.danger;
  }

  override toDOM(): Node {
    const el = document.createElement("span");
    el.className = `cm-run-marker${this.active ? " cm-run-marker-active" : ""}${this.danger ? " cm-run-marker-danger" : ""}`;
    el.textContent = "▶";
    el.setAttribute("aria-hidden", "true");
    el.title = this.danger ? "Run this statement (destructive)" : "Run this statement";
    return el;
  }
}

// WHAT:  Theme built from the CSS tokens so light/dark follow the app.
// WHERE: src/styles/globals.css (HeroUI variables)
const theme = EditorView.theme({
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)", height: "100%", fontSize: "13px" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--color-accent)", padding: "12px 0" },
  ".cm-line": { padding: "0 16px" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--color-muted)", border: "none", paddingRight: "4px" },
  ".cm-activeLine": { backgroundColor: "var(--color-surface-hover)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--foreground)" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--color-accent)", borderLeftWidth: "2px" },
  // WHAT:  Selection in the app's 10% accent wash, focused or not. The full
  //        descendant path mirrors CodeMirror's own base rule
  //        (`&light/dark.cm-focused > .cm-scroller > .cm-selectionLayer
  //        .cm-selectionBackground`), whose six-class specificity otherwise
  //        beats a short override — which is how the default light-lavender
  //        fill kept showing through on this dark editor.
  ".cm-selectionLayer .cm-selectionBackground": { backgroundColor: "var(--color-selection)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--color-selection)",
  },
  ".cm-panels": { backgroundColor: "var(--color-surface-elevated)", color: "var(--foreground)" },
  ".cm-tooltip": { backgroundColor: "var(--color-surface-elevated)", border: "1px solid var(--border)", color: "var(--foreground)", borderRadius: "4px" },
  ".cm-tooltip-autocomplete": { maxHeight: "200px" },
  // WHAT:  ScrollArea-style scrolling for the suggestion list: thin themed
  //        scrollbar, transparent track. A React ScrollArea cannot mount inside
  //        CodeMirror's own tooltip DOM, so the equivalent treatment lives here
  //        as theme rules (Chromium/WebView2 needs the -webkit- rules; the
  //        standard properties cover the rest).
  ".cm-tooltip-autocomplete > ul": {
    maxHeight: "200px",
    overflowY: "auto",
    overflowX: "hidden",
    scrollbarWidth: "thin",
    scrollbarColor: "var(--color-surface-tertiary) transparent",
  },
  ".cm-tooltip-autocomplete > ul::-webkit-scrollbar": { width: "8px" },
  ".cm-tooltip-autocomplete > ul::-webkit-scrollbar-track": { backgroundColor: "transparent" },
  ".cm-tooltip-autocomplete > ul::-webkit-scrollbar-thumb": { backgroundColor: "var(--color-surface-tertiary)", borderRadius: "4px" },
  ".cm-tooltip-autocomplete > ul > li": { padding: "2px 8px", cursor: "pointer" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--color-selection-strong)", color: "var(--foreground)" },
  ".cm-run-gutter": { minWidth: "16px", cursor: "pointer" },
  ".cm-run-marker": { display: "block", lineHeight: "inherit", textAlign: "center", fontSize: "9px", color: "var(--color-muted)", opacity: "0.35" },
  ".cm-run-gutter:hover .cm-run-marker": { opacity: "0.85" },
  ".cm-run-marker-active": { color: "var(--color-accent)", opacity: "1" },
  ".cm-run-marker-danger": { color: "var(--color-danger)", opacity: "0.8" },
  ".cm-active-statement": { backgroundColor: "var(--color-selection)" },
  // WHAT:  `{ dark: true }` tells CodeMirror this is a dark editor, so its own
  //        base rules resolve to the `&dark` variants (`#222`/`#233`) instead of
  //        the light ones (`#d9d9d9`/`#d7d4f0`) — the lavender wash that kept
  //        showing through selections on this black theme.
  // WHERE: @codemirror/view baseTheme (&light/&dark selectionBackground)
}, { dark: true });

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--color-syntax-keyword)", fontWeight: "600" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--color-syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--color-syntax-number)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--color-syntax-comment)", fontStyle: "italic" },
  { tag: [tags.operator, tags.punctuation], color: "var(--color-syntax-operator)" },
  { tag: [tags.typeName, tags.className, tags.standard(tags.name)], color: "var(--color-syntax-type)" },
]);

function sqlConfig(engine: Engine, schema: SQLNamespace, defaultSchema: string | undefined) {
  return sql(defaultSchema === undefined ? { dialect: dialectFor(engine), schema } : { dialect: dialectFor(engine), schema, defaultSchema });
}

function dialectFor(engine: Engine): SQLDialect {
  switch (engine) {
    case "postgres":
    case "supabase":
    case "neon":
    case "timescaledb":
    case "questdb":
    case "pgvector":
    case "postgis":
    case "cockroachdb":
    case "yugabytedb":
    case "duckdb":
    case "ibm_ims":
    case "raima_rdm":
      return PostgreSQL;
    case "mysql":
    case "planetscale":
    case "tidb":
      return MySQL;
    case "mariadb":
      return MariaSQL;
    case "mssql":
      return MSSQL;
    case "sqlite":
    case "spatialite":
    case "libsql":
    case "val_town":
    case "cloudflare_d1":
      return SQLite;
    default:
      return StandardSQL;
  }
}

// WHAT:  The gutter of ▶ markers, one per statement, on the line it starts.
// HOW:   Built once per editor and handed the caller's latest onRun through a ref,
//        so the extension never has to be rebuilt when the callback identity moves.
function runGutterFor(onRun: MutableRunHandler): Extension {
  return gutter({
    class: "cm-run-gutter",
    lineMarker: (view, line) => {
      const found = spanStartingAt(view.state, line.from);
      const span = found === -1 ? undefined : (view.state.field(spansField, false) ?? [])[found];
      if (span === undefined) return null;
      const target = targetOf(view.state);
      return new RunMarker(!target.selected && target.index === found + 1, span.intent === "destructive");
    },
    lineMarkerChange: (update) =>
      update.docChanged ||
      update.selectionSet ||
      update.startState.field(spansField, false) !== update.state.field(spansField, false),
    initialSpacer: () => new RunMarker(false, false),
    domEventHandlers: {
      mousedown: (view, line) => {
        const found = spanStartingAt(view.state, line.from);
        const all = view.state.field(spansField, false) ?? [];
        const span = found === -1 ? undefined : all[found];
        if (span === undefined) return false;
        const { from, to } = clampSpan(view.state, span);
        onRun.current({ text: view.state.doc.sliceString(from, to), from, to, index: found + 1, total: all.length, selected: false });
        return true;
      },
    },
  });
}

interface MutableRunHandler {
  current: (target: RunTarget) => void;
}

function wholeDoc(state: EditorState): RunTarget {
  return { text: state.doc.toString(), from: 0, to: state.doc.length, index: 0, total: 0, selected: false };
}

function sameTarget(a: RunTarget | null, b: RunTarget): boolean {
  return a !== null && a.from === b.from && a.to === b.to && a.selected === b.selected && a.index === b.index && a.total === b.total;
}

export function SqlEditor({ value, spans = NO_SPANS, onChange, onRun, onRunAll, onTargetChange, engine, schema, defaultSchema }: SqlEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  const onChangeRef = useRef(onChange);
  const onRunRef = useRef(onRun);
  const onRunAllRef = useRef(onRunAll);
  const onTargetRef = useRef(onTargetChange);
  const gutterCompartment = useRef(new Compartment());
  const runGutter = useRef<Extension | null>(null);
  const lastTarget = useRef<RunTarget | null>(null);
  useEffect(() => {
    onChangeRef.current = onChange;
    onRunRef.current = onRun;
    onRunAllRef.current = onRunAll;
    onTargetRef.current = onTargetChange;
  });

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const statementGutter = runGutterFor(onRunRef);
    runGutter.current = statementGutter;
    const announce = (state: EditorState) => {
      const target = targetOf(state);
      if (sameTarget(lastTarget.current, target)) return;
      lastTarget.current = target;
      onTargetRef.current?.(target);
    };
    const state = EditorState.create({
      doc: value,
      extensions: [
        spansField,
        activeStatement,
        gutterCompartment.current.of(spans.length > 0 ? statementGutter : []),
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        history(),
        bracketMatching(),
        closeBrackets(),
        autocompletion(),
        keymap.of([
          { key: "Mod-Enter", run: (view) => { onRunRef.current(targetOf(view.state)); return true; } },
          { key: "Mod-Shift-Enter", run: (view) => { (onRunAllRef.current ?? (() => onRunRef.current(wholeDoc(view.state))))(); return true; } },
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          ...completionKeymap,
          indentWithTab,
        ]),
        langCompartment.current.of(sqlConfig(engine, schema, defaultSchema)),
        syntaxHighlighting(highlight),
        theme,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          if (update.docChanged || update.selectionSet) announce(update.state);
        }),
      ],
    });
    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    announce(view.state);
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount once; value/schema/spans sync below
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: langCompartment.current.reconfigure(sqlConfig(engine, schema, defaultSchema)) });
  }, [engine, schema, defaultSchema]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
    }
  }, [value]);

  // The split runs in Rust a beat behind the typing; feeding the answer back in is
  // what moves the ▶ markers and re-reads which statement the caret is in.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: [setSpans.of(spans), gutterCompartment.current.reconfigure(spans.length > 0 ? (runGutter.current ?? []) : [])],
    });
    const target = targetOf(view.state);
    if (!sameTarget(lastTarget.current, target)) {
      lastTarget.current = target;
      onTargetRef.current?.(target);
    }
  }, [spans]);

  return <div ref={hostRef} className="h-full min-h-0 w-full overflow-hidden" />;
}
