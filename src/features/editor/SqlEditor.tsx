// SOT: sql-editor, codemirror-setup, editor-theme, completion-dropdown, run-target-gutter, statement-gutter, snippet-expansion, find-replace-host, error-squiggle-host, editor-handle, editor-shortcuts
import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState, type Ref } from "react";
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
  placeholder as placeholderExt,
  type DecorationSet,
  type KeyBinding as CmKeyBinding,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { closeBrackets, closeBracketsKeymap, snippet } from "@codemirror/autocomplete";
import type { Completion } from "@codemirror/autocomplete";
import { HighlightStyle, StreamLanguage, syntaxHighlighting, bracketMatching, type StreamParser } from "@codemirror/language";
import { sql, MSSQL, MySQL, MariaSQL, PostgreSQL, SQLite, StandardSQL, type SQLDialect, type SQLNamespace } from "@codemirror/lang-sql";
import { tags } from "@lezer/highlight";
import type { Engine, RunScope, Snippet, StatementSpan } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { queryLanguageOf, type QueryLanguage } from "@/lib/queryLanguage";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Icon } from "@/lib/icons";
import type { IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { matchesChord, toCodeMirrorKey, type Keymap } from "@/lib/keymap";
import { useKeymap } from "@/stores/useShortcut";
import { useWorkspace } from "@/stores/workspace";
import { useResolvedTheme, type ResolvedTheme } from "@/stores/useTheme";
import { snippetForWord, snippetLanguageOf, snippetsFor } from "@/lib/snippets";
import { isRedisEngine } from "@/lib/redisCommands";
import { catalogFor } from "@/lib/sqlCatalog";
import { computeCompletions, type CompletionItem, type CompletionKind, type EditorLanguage, type SchemaTable, type TemplateVariable } from "./completion";
import { diagnosticsExtension, setDiagnostic, type EditorDiagnostic } from "./diagnostics";
import { FindBar } from "./FindBar";
import { EMPTY_STATUS, findExtension, findField, replaceAll, replaceCurrent, selectNearest, setFind, statusOf, stepFind, type FindQuery, type FindStatus } from "./findReplace";
import { planRun, sameTarget, type Range, type RunTarget } from "./runTargets";
import { mentionedTables, statementAround, tokenize } from "./sqlScan";

export type { RunTarget } from "./runTargets";
export type { EditorDiagnostic } from "./diagnostics";
export type { TemplateVariable } from "./completion";

// WHAT:  What a parent can ask of the editor without owning its state.
// WHY:   The inline AI prompt needs the caret's place on screen and a way to put
//        its answer into the text as one undoable edit; the results pane needs to
//        jump to an error. None of that belongs in props that re-render the editor.
export interface SqlEditorHandle {
  focus: () => void;
  /// The main selection and the text it covers.
  selection: () => { from: number; to: number; text: string };
  /// Where the caret is, relative to the editor's box (null while it is scrolled out of view).
  caretBox: () => { left: number; top: number; bottom: number; width: number; height: number } | null;
  /// Replaces `from..to` as one undoable edit; selects what went in unless `select` is false.
  replace: (from: number, to: number, insert: string, select?: boolean) => void;
  /// The statement a "Run statement" would send now: the highlighted text, else the one the caret is in; null with none.
  statementRange: () => { from: number; to: number } | null;
  /// Moves the caret (or selects `from..to`) and scrolls it into view.
  reveal: (from: number, to?: number) => void;
  openFind: (withReplace: boolean) => void;
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
  /// What Run does with a bare caret: every statement, or the one it is in.
  runScope?: RunScope | undefined;
  /// `{{names}}` the surrounding page defines, offered inside `{{ }}`.
  templateVars?: readonly TemplateVariable[] | undefined;
  /// Where to draw the squiggle for the last error; a new object draws it again.
  diagnostic?: EditorDiagnostic | null | undefined;
  /// The tables the caret's statement names, so their columns can be fetched.
  onMentionedTables?: ((names: string[]) => void) | undefined;
  /// The editor's shortcuts for features its parent owns. A chord is only
  /// claimed when the parent passes the callback, so a dashboard's ⌘S still saves the dashboard.
  onInlineAi?: (() => void) | undefined;
  onSave?: (() => void) | undefined;
  onOpenFile?: (() => void) | undefined;
  handleRef?: Ref<SqlEditorHandle> | undefined;
}

const NO_SPANS: readonly StatementSpan[] = [];
const NO_VARIABLES: readonly TemplateVariable[] = [];

const setSpans = StateEffect.define<readonly StatementSpan[]>();
const setScope = StateEffect.define<RunScope>();

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

/// Run Without a Selection (Settings → Editor), as the editor holds it.
const scopeField = StateField.define<RunScope>({
  create: () => "current",
  update(scope, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setScope)) return effect.value;
    }
    return scope;
  },
});

// WHAT:  Keeps a span inside the document.
// WHY:   Spans are computed a beat behind the typing that produced them; an offset
//        past the end of the doc makes `lineAt` throw rather than merely look wrong.
function clampSpan(state: EditorState, span: StatementSpan): Range {
  const max = state.doc.length;
  const from = Math.min(Math.max(span.start, 0), max);
  return { from, to: Math.min(Math.max(span.end, from), max) };
}

// WHAT:  What ⌘/Ctrl + Enter, the toolbar Run and the gutter ▶ would send.
// HOW:   See `planRun`: a highlighted range runs as highlighted, several carets or
//        selections run the statements they touch, a bare caret runs its statement
//        or the whole buffer by the run-scope setting.
function targetOf(state: EditorState): RunTarget {
  const spans = state.field(spansField, false) ?? [];
  const scope = state.field(scopeField, false) ?? "current";
  return planRun(spans, state.selection.ranges.map((r) => ({ from: r.from, to: r.to })), state.doc.length, scope);
}

function wholeTarget(state: EditorState): RunTarget {
  const spans = state.field(spansField, false) ?? [];
  return { kind: "all", pieces: [{ from: 0, to: state.doc.length }], count: Math.max(spans.length, 1), index: 0, total: spans.length };
}

/// The first line of a statement, which is where its ▶ sits.
function markerLine(state: EditorState, span: StatementSpan): number {
  return state.doc.lineAt(clampSpan(state, span).from).from;
}

function spanStartingAt(state: EditorState, lineFrom: number): number {
  return (state.field(spansField, false) ?? []).findIndex((s) => markerLine(state, s) === lineFrom);
}

const statementMark = Decoration.mark({ class: "cm-active-statement" });

// WHAT:  Tints what Run will send when that is a statement or several, so it is
//        visible before it is sent. Only worth drawing when the buffer holds more than one.
function decorate(state: EditorState): DecorationSet {
  const target = targetOf(state);
  if ((target.kind !== "statement" && target.kind !== "several") || target.total < 2) return Decoration.none;
  return Decoration.set(target.pieces.filter((p) => p.to > p.from).map((p) => statementMark.range(p.from, p.to)));
}

const activeStatement = StateField.define<DecorationSet>({
  create: (state) => decorate(state),
  update: (deco, tr) =>
    tr.docChanged || tr.selection || tr.effects.some((e) => e.is(setSpans) || e.is(setScope)) ? decorate(tr.state) : deco,
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
// WHERE: src/styles/globals.css (design tokens), src/stores/useTheme.ts
const THEME_SPEC: Parameters<typeof EditorView.theme>[0] = {
  // Settings → Fonts → Editor font size: App.tsx publishes it as --editor-font-size.
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)", height: "100%", fontSize: "var(--editor-font-size, 13px)" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--color-accent)", padding: "6px 0" },
  ".cm-line": { padding: "0 12px" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--color-muted)", border: "none", paddingRight: "4px" },
  ".cm-activeLine": { backgroundColor: "var(--color-surface-hover)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--foreground)" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--color-accent)", borderLeftWidth: "2px" },
  // WHAT:  Selection in the app's accent wash, focused or not. The full
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
  ".cm-placeholder": { color: "var(--color-muted)", opacity: "0.55", fontStyle: "normal" },
  ".cm-run-gutter": { minWidth: "16px", cursor: "pointer" },
  ".cm-run-marker": { display: "block", lineHeight: "inherit", textAlign: "center", fontSize: "9px", color: "var(--color-muted)", opacity: "0.35" },
  ".cm-run-gutter:hover .cm-run-marker": { opacity: "0.85" },
  ".cm-run-marker-active": { color: "var(--color-accent)", opacity: "1" },
  ".cm-run-marker-danger": { color: "var(--color-danger)", opacity: "0.8" },
  ".cm-active-statement": { backgroundColor: "var(--color-selection)" },
  // Find and replace: every match softly, the current one firmly.
  ".cm-find-match": { backgroundColor: "var(--color-warning-soft)", borderRadius: "2px" },
  ".cm-find-current": { backgroundColor: "var(--color-warning-soft)", outline: "1px solid var(--color-warning)", borderRadius: "2px" },
  // The error the last run pointed at.
  ".cm-diagnostic-error": { textDecoration: "underline wavy var(--color-danger)", textDecorationSkipInk: "none", textUnderlineOffset: "3px" },
  ".cm-diagnostic-card": { padding: "6px 10px", maxWidth: "420px", fontSize: "12px", lineHeight: "1.45", whiteSpace: "pre-wrap" },
  ".cm-diagnostic-message": { color: "var(--color-danger)", fontWeight: "600" },
  ".cm-diagnostic-hint, .cm-diagnostic-detail": { color: "var(--color-muted)", marginTop: "3px" },
};

// WHAT:  `{ dark }` tells CodeMirror which of its own base rules to use: the
//        `&dark` variants (`#222`/`#233`) on the black theme, the `&light` ones
//        on the light theme. A dark editor flagged light let the lavender
//        `#d7d4f0` wash show through selections; the flag now follows the
//        resolved app theme (reconfigured through a Compartment).
// WHERE: @codemirror/view baseTheme (&light/&dark selectionBackground)
const DARK_THEME = EditorView.theme(THEME_SPEC, { dark: true });
const LIGHT_THEME = EditorView.theme(THEME_SPEC, { dark: false });

function editorTheme(theme: ResolvedTheme): Extension {
  return theme === "light" ? LIGHT_THEME : DARK_THEME;
}

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--color-syntax-keyword)", fontWeight: "600" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--color-syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: "var(--color-syntax-number)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--color-syntax-comment)", fontStyle: "italic" },
  { tag: [tags.operator, tags.punctuation], color: "var(--color-syntax-operator)" },
  { tag: [tags.typeName, tags.className, tags.standard(tags.name), tags.variableName], color: "var(--color-syntax-type)" },
]);

// WHAT:  The highlighter for the connection's own language. SQL engines get
//        lang-sql with their dialect; everything else gets a small stream
//        tokenizer that knows its comment marker, strings, numbers and the
//        console's own verbs, so `HGETALL`, `$match` or `MATCH (n)` read as
//        keywords instead of plain text.
// WHERE: src/lib/queryLanguage.ts (per-family vocabulary)
function languageConfig(engine: Engine, schema: SQLNamespace, defaultSchema: string | undefined): Extension {
  const language = queryLanguageOf(engine);
  if (language.syntax === "sql") {
    return sql(defaultSchema === undefined ? { dialect: dialectFor(engine), schema } : { dialect: dialectFor(engine), schema, defaultSchema });
  }
  return StreamLanguage.define(nativeTokenizer(language));
}

interface NativeState {
  /// Inside an XQuery `(: … :)` comment that spans lines.
  comment: boolean;
}

function nativeTokenizer(language: QueryLanguage): StreamParser<NativeState> {
  const words = new Set(language.keywords.map((k) => k.toLowerCase()));
  const xquery = language.comment === "(:";
  return {
    name: language.name,
    startState: () => ({ comment: false }),
    token(stream, state) {
      if (state.comment) {
        if (stream.skipTo(":)")) {
          stream.match(":)");
          state.comment = false;
        } else stream.skipToEnd();
        return "comment";
      }
      if (stream.eatSpace()) return null;
      if (xquery && stream.match("(:")) {
        state.comment = true;
        return "comment";
      }
      // Comments start a line (after indentation): `#` mid-line is part of a
      // key (`user#1`) and `//` part of a path or URL.
      if (!xquery && stream.column() === stream.indentation() && stream.match(language.comment)) {
        stream.skipToEnd();
        return "comment";
      }
      const ch = stream.peek() ?? "";
      if (ch === '"' || ch === "'" || ch === "`") {
        stream.next();
        let escaped = false;
        for (let next = stream.next(); next !== undefined; next = stream.next()) {
          if (next === ch && !escaped) break;
          escaped = !escaped && next === "\\";
        }
        return "string";
      }
      if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) return "number";
      if (stream.match(/^[$?]?[A-Za-z_][\w.:$-]*/)) {
        const word = stream.current().toLowerCase();
        if (word === "true" || word === "false" || word === "null") return "atom";
        if (words.has(word)) return "keyword";
        return word.startsWith("$") || word.startsWith("?") ? "variableName" : null;
      }
      if (stream.match(/^[{}[\](),:;|=<>!+*/%.-]/)) return "operator";
      stream.next();
      return null;
    },
  };
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

interface MutableRunHandler {
  current: (target: RunTarget) => void;
}

interface MutableRunAllHandler {
  current: (() => void) | undefined;
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
      const mine = clampSpan(view.state, span);
      const active = (target.kind === "statement" || target.kind === "several") && target.pieces.some((p) => p.from === mine.from && p.to === mine.to);
      return new RunMarker(active, span.intent === "destructive");
    },
    lineMarkerChange: (update) =>
      update.docChanged ||
      update.selectionSet ||
      update.startState.field(spansField, false) !== update.state.field(spansField, false) ||
      update.startState.field(scopeField, false) !== update.state.field(scopeField, false),
    initialSpacer: () => new RunMarker(false, false),
    domEventHandlers: {
      mousedown: (view, line) => {
        const found = spanStartingAt(view.state, line.from);
        const all = view.state.field(spansField, false) ?? [];
        const span = found === -1 ? undefined : all[found];
        if (span === undefined) return false;
        onRun.current({ kind: "statement", pieces: [clampSpan(view.state, span)], count: 1, index: found + 1, total: all.length });
        return true;
      },
    },
  });
}

// WHAT:  Run and Run all on the chords the user bound them to (Settings → Shortcuts).
// WHY:   These two live inside CodeMirror, not on a window listener, because the
//        editor must consume the key before its own Enter handling inserts a newline.
// HOW:   Lives in a Compartment; a rebind reconfigures it without rebuilding the
//        editor. An unbound action (empty chord) simply has no entry.
// WHERE: src/lib/keymap.ts (toCodeMirrorKey), src/stores/useShortcut.ts (useKeymap)
function runKeysFor(keys: Keymap, onRun: MutableRunHandler, onRunAll: MutableRunAllHandler): Extension {
  const bindings: CmKeyBinding[] = [];
  const run = toCodeMirrorKey(keys.run);
  if (run !== null) bindings.push({ key: run, run: (view) => { onRun.current(targetOf(view.state)); return true; } });
  const runAll = toCodeMirrorKey(keys["run-all"]);
  if (runAll !== null) bindings.push({ key: runAll, run: (view) => { (onRunAll.current ?? (() => onRun.current(wholeTarget(view.state))))(); return true; } });
  return [keymap.of(bindings), claimChords([keys.run, keys["run-all"]])];
}

// WHAT:  Stops a key the editor handled from also reaching the app's window-level
//        shortcuts.
// WHY:   ⌘S saves the query here and commits staged grid changes elsewhere, ⌘F
//        finds here and in grids: a chord the focused editor owns must not fire
//        the other.
function claimChords(chords: readonly string[]): Extension {
  const owned = chords.filter((c) => c.length > 0);
  return EditorView.domEventHandlers({
    keydown: (event) => {
      if (owned.some((chord) => matchesChord(event, chord))) event.stopPropagation();
      return false;
    },
  });
}

interface EditorActions {
  inlineAi: (() => void) | undefined;
  save: (() => void) | undefined;
  openFile: (() => void) | undefined;
  openFind: (withReplace: boolean) => void;
  nextMatch: () => void;
  previousMatch: () => void;
}

interface MutableActions {
  current: EditorActions;
}

// WHAT:  The editor's own shortcuts: find, find next / previous, inline AI, save, open file.
// WHY:   They are rebindable (Settings → Shortcuts) and ahead of the default keymap,
//        which would otherwise take ⌘I for "select parent syntax".
// HOW:   A chord is bound only when its callback exists: a dashboard tile's editor
//        has no query to save, so ⌘S there goes on to the dashboard's own handler.
function editorKeysFor(keys: Keymap, actions: MutableActions): Extension {
  const bindings: CmKeyBinding[] = [];
  const claimed: string[] = [];
  const bind = (chord: string, run: () => boolean) => {
    const key = toCodeMirrorKey(chord);
    if (key === null) return;
    bindings.push({ key, run, preventDefault: true });
    claimed.push(chord);
  };
  bind(keys["find-in-query"], () => {
    actions.current.openFind(false);
    return true;
  });
  if (actions.current.inlineAi !== undefined) {
    bind(keys["inline-ai"], () => {
      actions.current.inlineAi?.();
      return true;
    });
  }
  if (actions.current.save !== undefined) {
    bind(keys["save-query"], () => {
      actions.current.save?.();
      return true;
    });
  }
  if (actions.current.openFile !== undefined) {
    bind(keys["open-sql-file"], () => {
      actions.current.openFile?.();
      return true;
    });
  }
  bindings.push(
    { key: "Mod-Alt-f", run: () => { actions.current.openFind(true); return true; }, preventDefault: true },
    { key: "Mod-g", run: () => { actions.current.nextMatch(); return true; }, preventDefault: true },
    { key: "Shift-Mod-g", run: () => { actions.current.previousMatch(); return true; }, preventDefault: true },
  );
  return [keymap.of(bindings), claimChords(claimed)];
}

type Language = EditorLanguage;

function languageOf(engine: Engine): Language {
  if (catalogFor(engine) !== null) return "sql";
  if (isRedisEngine(engine)) return "redis";
  return ENGINE_FACTS[engine].family === "mongodb" ? "mongo" : "other";
}

interface CompletionSnapshot {
  x: number;
  y: number;
  from: number;
  to: number;
  items: CompletionItem[];
  index: number;
}

const COMPLETION_ICONS: Record<CompletionKind, IconName> = {
  snippet: "flash",
  table: "table",
  column: "columns",
  keyword: "code",
  function: "function",
  variable: "braces",
  command: "terminal",
  method: "function",
};
const NO_SNIPPETS: readonly Snippet[] = [];

/// The characters a snippet prefix may use; the run of them before the caret is
/// what Tab tries to expand.
const SNIPPET_WORD = /[A-Za-z0-9_.-]+$/;

// WHAT:  Replaces `from..to` with a snippet body and selects its first field.
// HOW:   @codemirror/autocomplete's `snippet()` installs its own field state and
//        a highest-precedence Tab / Shift-Tab / Esc keymap on first use, so Tab
//        walks the fields without anything else registered here.
function expandSnippet(view: EditorView, body: string, from: number, to: number): void {
  snippet(body)(view, null, from, to);
}

// WHAT:  Flattens the catalogue namespace into tables. Only the shapes the
//        app's own builder emits are walked (`{ table: [...] }` and
//        `{ schema: { table: [...] } }`, plus the `{ self, children }`
//        completion-node form); anything else degrades to no suggestions
//        rather than a crash. A level holding both a `self` and a `children`
//        key is read as the node form, so a schema with literal tables of
//        those names would misread — vanishingly rare, and it only costs
//        suggestions, never correctness.
// WHERE: src/features/editor/QueryPane.tsx (buildNamespace)
function tablesOf(schema: SQLNamespace): SchemaTable[] {
  const out: SchemaTable[] = [];
  collectTables(schema, "", out);
  return out;
}

function isCompletionList(node: SQLNamespace): node is readonly (Completion | string)[] {
  return Array.isArray(node);
}

function collectTables(node: SQLNamespace, prefix: string, out: SchemaTable[]): void {
  // WHAT:  A declared guard type, not Array.isArray inline: narrowing a
  //        `readonly` array member out of this union needs the predicate's own
  //        type — the lib guard leaves the member behind and indexing then
  //        fails.
  if (isCompletionList(node)) return;
  if ("children" in node && "self" in node) {
    collectTables(node.children, prefix, out);
    return;
  }
  for (const name of Object.keys(node)) {
    const value = node[name];
    if (value === undefined) continue;
    const label = prefix.length > 0 ? `${prefix}.${name}` : name;
    if (Array.isArray(value)) {
      out.push({ name: label, columns: value.filter((c): c is string => typeof c === "string") });
    } else {
      collectTables(value, label, out);
    }
  }
}

// WHAT:  The text suggestions are computed from: the statement around the caret
//        when the buffer's statements are known, else the whole text.
// WHY:   Scanning a 5,000-line script on every keystroke to learn which clause
//        one statement is in is wasted work; the Rust splitter has already said
//        where each statement is.
function windowOf(state: EditorState): { text: string; caret: number; base: number } {
  const caret = state.selection.main.head;
  const spans = (state.field(spansField, false) ?? []).map((s) => clampSpan(state, s));
  if (spans.length === 0) return { text: state.doc.toString(), caret, base: 0 };
  const inside = spans.find((r) => caret >= r.from && caret <= r.to);
  let from: number;
  let to: number;
  if (inside !== undefined) {
    from = inside.from;
    to = inside.to;
  } else {
    // Between statements: the gap is a new statement being typed.
    const before = spans.filter((r) => r.to < caret);
    from = before.length > 0 ? (before[before.length - 1]?.to ?? 0) : 0;
    to = spans.find((r) => r.from > caret)?.from ?? state.doc.length;
  }
  return { text: state.doc.sliceString(from, to), caret: caret - from, base: from };
}

export function SqlEditor({
  value,
  spans = NO_SPANS,
  onChange,
  onRun,
  onRunAll,
  onTargetChange,
  engine,
  schema,
  defaultSchema,
  runScope = "current",
  templateVars = NO_VARIABLES,
  diagnostic = null,
  onMentionedTables,
  onInlineAi,
  onSave,
  onOpenFile,
  handleRef,
}: SqlEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  const onChangeRef = useRef(onChange);
  const onRunRef = useRef(onRun);
  const onRunAllRef = useRef(onRunAll);
  const onTargetRef = useRef(onTargetChange);
  const onMentionedRef = useRef(onMentionedTables);
  const gutterCompartment = useRef(new Compartment());
  const runKeysCompartment = useRef(new Compartment());
  const editorKeysCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  const appTheme = useResolvedTheme();
  const appThemeRef = useRef(appTheme);
  const keys = useKeymap();
  const keysRef = useRef(keys);
  const runGutter = useRef<Extension | null>(null);
  const lastTarget = useRef<RunTarget | null>(null);
  const [completion, setCompletion] = useState<CompletionSnapshot | null>(null);
  const completionRef = useRef<CompletionSnapshot | null>(null);
  const tables = useMemo(() => tablesOf(schema), [schema]);
  const tablesRef = useRef(tables);
  const language = useMemo(() => languageOf(engine), [engine]);
  const languageRef = useRef(language);
  const engineRef = useRef(engine);
  const catalog = useMemo(() => catalogFor(engine), [engine]);
  const catalogRef = useRef(catalog);
  const variablesRef = useRef(templateVars);
  // WHAT:  Snippets for this editor's language: the user's, then the built-ins.
  // WHERE: src/lib/snippets.ts, AppSettings.snippets
  const userSnippets = useWorkspace((s) => s.settings?.snippets ?? NO_SNIPPETS);
  const snippets = useMemo(() => snippetsFor(snippetLanguageOf(engine), userSnippets), [engine, userSnippets]);
  const snippetsRef = useRef(snippets);
  const defaultSchemaRef = useRef(defaultSchema);
  // The connection's own language: the highlighter's vocabulary and the example an
  // empty tab shows. Follows the engine if the tab is pointed at another connection.
  const nativeLanguage = queryLanguageOf(engine);
  const placeholderCompartment = useRef(new Compartment());
  const suppressCompletionRef = useRef(false);
  const mentionTimer = useRef<number | null>(null);
  const lastMentioned = useRef("");
  const [find, setFindOpen] = useState<{ key: number; initial: string; replace: boolean } | null>(null);
  const [findStatus, setFindStatus] = useState<FindStatus>(EMPTY_STATUS);
  const findCounter = useRef(0);
  const actionsRef = useRef<EditorActions>({ inlineAi: onInlineAi, save: onSave, openFile: onOpenFile, openFind: () => undefined, nextMatch: () => undefined, previousMatch: () => undefined });
  const editorFontSize = useWorkspace((s) => s.settings?.editorFontSize);
  const editorFont = useWorkspace((s) => s.settings?.editorFont);

  useEffect(() => {
    onChangeRef.current = onChange;
    onRunRef.current = onRun;
    onRunAllRef.current = onRunAll;
    onTargetRef.current = onTargetChange;
    onMentionedRef.current = onMentionedTables;
    tablesRef.current = tables;
    languageRef.current = language;
    engineRef.current = engine;
    catalogRef.current = catalog;
    variablesRef.current = templateVars;
    defaultSchemaRef.current = defaultSchema;
    snippetsRef.current = snippets;
  });

  const hideCompletion = useCallback(() => {
    completionRef.current = null;
    setCompletion(null);
  }, []);

  const acceptCompletion = useCallback((view: EditorView): boolean => {
    const snapshot = completionRef.current;
    const item = snapshot === null ? undefined : snapshot.items[snapshot.index];
    if (snapshot === null || item === undefined) return false;
    suppressCompletionRef.current = true;
    if (item.snippet !== undefined) {
      expandSnippet(view, item.snippet.body, snapshot.from, snapshot.to);
    } else {
      const insert = item.insert ?? item.label;
      view.dispatch({
        changes: { from: snapshot.from, to: snapshot.to, insert },
        selection: { anchor: snapshot.from + (item.caret ?? insert.length) },
      });
    }
    completionRef.current = null;
    setCompletion(null);
    view.focus();
    return true;
  }, []);

  // WHAT:  Tab on a word that is exactly a snippet prefix expands it (`sel⇥`).
  // WHY:   Checked before accepting a suggestion: the dropdown ranks `SELECT`
  //        and friends alongside, and the exact prefix is what was meant.
  const tabExpand = useCallback((view: EditorView): boolean => {
    const selection = view.state.selection.main;
    if (!selection.empty) return false;
    const line = view.state.doc.lineAt(selection.head);
    const word = SNIPPET_WORD.exec(line.text.slice(0, selection.head - line.from))?.[0] ?? "";
    const found = snippetForWord(word, snippetsRef.current);
    if (found === undefined) return false;
    suppressCompletionRef.current = true;
    expandSnippet(view, found.body, selection.head - word.length, selection.head);
    completionRef.current = null;
    setCompletion(null);
    return true;
  }, []);

  const moveCompletion = useCallback((host: HTMLDivElement | null, delta: number): boolean => {
    const snapshot = completionRef.current;
    if (snapshot === null || snapshot.items.length === 0) return false;
    const index = (snapshot.index + delta + snapshot.items.length) % snapshot.items.length;
    const next: CompletionSnapshot = { ...snapshot, index };
    completionRef.current = next;
    setCompletion(next);
    host?.querySelector(`[data-completion-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
    return true;
  }, []);

  // WHAT:  Recomputes the suggestion dropdown from the caret context after
  //        every edit or move. Positioning is caret-anchored and clamped to
  //        the editor box, flipping above the caret when there is no room below.
  const refreshCompletion = useCallback((view: EditorView) => {
    // The accept transaction itself fires this listener; skipping once keeps
    // the just-inserted word from reopening the list on the same tick.
    if (suppressCompletionRef.current) {
      suppressCompletionRef.current = false;
      return;
    }
    const host = hostRef.current;
    if (host === null) return;
    const state = view.state;
    const found = state.selection.main.empty
      ? (() => {
          const scope = windowOf(state);
          const result = computeCompletions({
            text: scope.text,
            caret: scope.caret,
            engine: engineRef.current,
            tables: tablesRef.current,
            catalog: catalogRef.current,
            language: languageRef.current,
            snippets: snippetsRef.current,
            defaultSchema: defaultSchemaRef.current,
            variables: variablesRef.current,
          });
          return result === null ? null : { ...result, from: result.from + scope.base, to: result.to + scope.base };
        })()
      : null;
    if (found === null) {
      completionRef.current = null;
      setCompletion(null);
      return;
    }
    const coords = view.coordsAtPos(state.selection.main.head);
    if (coords === null) {
      completionRef.current = null;
      setCompletion(null);
      return;
    }
    const box = host.getBoundingClientRect();
    const width = 256;
    const listHeight = 224;
    const x = Math.max(0, Math.min(coords.left - box.left, Math.max(0, box.width - width - 8)));
    const below = coords.bottom - box.top + 4;
    const y = below + listHeight > box.height && coords.top - box.top > listHeight + 8 ? Math.max(0, coords.top - box.top - listHeight - 4) : below;
    const snapshot: CompletionSnapshot = { x, y, from: found.from, to: found.to, items: found.items, index: 0 };
    completionRef.current = snapshot;
    setCompletion(snapshot);
  }, []);

  // WHAT:  Tells the parent which tables the caret's statement names, shortly
  //        after typing stops, so their columns are on hand when `alias.` is typed.
  const announceMentions = useCallback((view: EditorView) => {
    if (mentionTimer.current !== null) window.clearTimeout(mentionTimer.current);
    mentionTimer.current = window.setTimeout(() => {
      const callback = onMentionedRef.current;
      if (callback === undefined || languageRef.current !== "sql") return;
      const scope = windowOf(view.state);
      const bounds = statementAround(scope.text, scope.caret);
      const names = mentionedTables(tokenize(scope.text.slice(bounds.from, bounds.to))).map((t) => t.name);
      const signature = names.join("|");
      if (signature === lastMentioned.current) return;
      lastMentioned.current = signature;
      callback(names);
    }, 300);
  }, []);

  // ---- find / replace ------------------------------------------------------------

  const openFind = useCallback((withReplace: boolean) => {
    const view = viewRef.current;
    const selection = view?.state.selection.main;
    const picked = view !== null && selection !== undefined && !selection.empty && selection.to - selection.from < 200 ? view.state.sliceDoc(selection.from, selection.to) : "";
    findCounter.current += 1;
    setFindOpen({ key: findCounter.current, initial: picked.includes("\n") ? "" : picked, replace: withReplace });
  }, []);

  const closeFind = useCallback(() => {
    const view = viewRef.current;
    view?.dispatch({ effects: setFind.of(null) });
    setFindOpen(null);
    setFindStatus(EMPTY_STATUS);
    view?.focus();
  }, []);

  const changeFindQuery = useCallback((query: FindQuery) => {
    const view = viewRef.current;
    if (view === null) return;
    view.dispatch({ effects: setFind.of(query.search.length === 0 ? null : query) });
    if (query.search.length > 0) selectNearest(view);
  }, []);

  // The callbacks are read when a chord fires, so they can change every render.
  useEffect(() => {
    actionsRef.current = {
      inlineAi: onInlineAi,
      save: onSave,
      openFile: onOpenFile,
      openFind,
      nextMatch: () => {
        const view = viewRef.current;
        if (view !== null) stepFind(view, 1);
      },
      previousMatch: () => {
        const view = viewRef.current;
        if (view !== null) stepFind(view, -1);
      },
    };
  });

  // Which chords the editor claims depends on which callbacks the parent supplies.
  const hasInlineAi = onInlineAi !== undefined;
  const hasSave = onSave !== undefined;
  const hasOpenFile = onOpenFile !== undefined;
  useEffect(() => {
    viewRef.current?.dispatch({ effects: editorKeysCompartment.current.reconfigure(editorKeysFor(keysRef.current, actionsRef)) });
  }, [hasInlineAi, hasSave, hasOpenFile]);

  useImperativeHandle(
    handleRef,
    (): SqlEditorHandle => ({
      focus: () => viewRef.current?.focus(),
      selection: () => {
        const view = viewRef.current;
        const main = view?.state.selection.main;
        if (view === null || main === undefined) return { from: 0, to: 0, text: "" };
        return { from: main.from, to: main.to, text: view.state.sliceDoc(main.from, main.to) };
      },
      caretBox: () => {
        const view = viewRef.current;
        const host = hostRef.current;
        if (view === null || host === null) return null;
        const coords = view.coordsAtPos(view.state.selection.main.head);
        if (coords === null) return null;
        const box = host.getBoundingClientRect();
        return { left: coords.left - box.left, top: coords.top - box.top, bottom: coords.bottom - box.top, width: box.width, height: box.height };
      },
      replace: (from, to, insert, select = true) => {
        const view = viewRef.current;
        if (view === null) return;
        const max = view.state.doc.length;
        const start = Math.min(Math.max(from, 0), max);
        const end = Math.min(Math.max(to, start), max);
        view.dispatch({
          changes: { from: start, to: end, insert },
          selection: select ? { anchor: start, head: start + insert.length } : { anchor: start + insert.length },
          scrollIntoView: true,
        });
        view.focus();
      },
      statementRange: () => {
        const view = viewRef.current;
        if (view === null) return null;
        const spans = view.state.field(spansField, false) ?? [];
        const picked = planRun(spans, view.state.selection.ranges.map((r) => ({ from: r.from, to: r.to })), view.state.doc.length, "current");
        const first = picked.pieces[0];
        return picked.kind === "all" || first === undefined ? null : { from: first.from, to: picked.pieces[picked.pieces.length - 1]?.to ?? first.to };
      },
      reveal: (from, to) => {
        const view = viewRef.current;
        if (view === null) return;
        const max = view.state.doc.length;
        const start = Math.min(Math.max(from, 0), max);
        view.dispatch({ selection: { anchor: start, head: Math.min(Math.max(to ?? start, start), max) }, scrollIntoView: true });
        view.focus();
      },
      openFind,
    }),
    [openFind],
  );

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
        // Several carets or selections (Alt/⌘-click) each run the statement they touch.
        EditorState.allowMultipleSelections.of(true),
        spansField,
        scopeField,
        activeStatement,
        findExtension(),
        diagnosticsExtension(),
        gutterCompartment.current.of(spans.length > 0 ? statementGutter : []),
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        history(),
        bracketMatching(),
        closeBrackets(),
        runKeysCompartment.current.of(runKeysFor(keysRef.current, onRunRef, onRunAllRef)),
        editorKeysCompartment.current.of(editorKeysFor(keysRef.current, actionsRef)),
        keymap.of([
          { key: "Escape", run: () => { if (completionRef.current === null) return false; hideCompletion(); return true; } },
          { key: "ArrowDown", run: () => moveCompletion(host, 1) },
          { key: "ArrowUp", run: () => moveCompletion(host, -1) },
          { key: "Enter", run: (view) => acceptCompletion(view) },
          { key: "Tab", run: (view) => tabExpand(view) || acceptCompletion(view) },
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...historyKeymap,
          indentWithTab,
        ]),
        langCompartment.current.of(languageConfig(engine, schema, defaultSchema)),
        placeholderCompartment.current.of(placeholderExt(nativeLanguage.example)),
        syntaxHighlighting(highlight),
        themeCompartment.current.of(editorTheme(appThemeRef.current)),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          if (update.docChanged || update.selectionSet) {
            announce(update.state);
            refreshCompletion(update.view);
            announceMentions(update.view);
          }
          if (update.state.field(findField, false) !== update.startState.field(findField, false)) setFindStatus(statusOf(update.view));
        }),
      ],
    });
    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    view.dispatch({ effects: setScope.of(runScope) });
    announce(view.state);
    // A React ScrollArea cannot mount inside CodeMirror's own tooltip DOM, so
    // suggestions render in the app's own dropdown below — which means scroll
    // and blur have to dismiss it explicitly.
    const hide = () => hideCompletion();
    const scroller = host.querySelector(".cm-scroller");
    scroller?.addEventListener("scroll", hide);
    view.dom.addEventListener("focusout", hide);
    return () => {
      scroller?.removeEventListener("scroll", hide);
      view.dom.removeEventListener("focusout", hide);
      if (mentionTimer.current !== null) window.clearTimeout(mentionTimer.current);
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount once; value/schema/spans sync below
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: [
        langCompartment.current.reconfigure(languageConfig(engine, schema, defaultSchema)),
        placeholderCompartment.current.reconfigure(placeholderExt(queryLanguageOf(engine).example)),
      ],
    });
  }, [engine, schema, defaultSchema]);

  // Settings → Themes (or the OS, on "system") reaches an open editor too.
  useEffect(() => {
    appThemeRef.current = appTheme;
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: themeCompartment.current.reconfigure(editorTheme(appTheme)) });
  }, [appTheme]);

  // The font and its size reach the editor through CSS variables; the editor only
  // has to be told its lines are a different height now.
  useEffect(() => {
    viewRef.current?.requestMeasure();
  }, [editorFontSize, editorFont]);

  // A rebind in Settings reaches an open editor without remounting it.
  useEffect(() => {
    keysRef.current = keys;
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: [
        runKeysCompartment.current.reconfigure(runKeysFor(keys, onRunRef, onRunAllRef)),
        editorKeysCompartment.current.reconfigure(editorKeysFor(keys, actionsRef)),
      ],
    });
  }, [keys]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: setScope.of(runScope) });
    const target = targetOf(view.state);
    if (!sameTarget(lastTarget.current, target)) {
      lastTarget.current = target;
      onTargetRef.current?.(target);
    }
  }, [runScope]);

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

  // The error the last run pointed at. A new object draws it again; any edit clears it.
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({ effects: setDiagnostic.of(diagnostic) });
  }, [diagnostic]);

  // WHAT:  The suggestion dropdown: the app's own floating panel with a
  //        shadcn ScrollArea, caret-anchored. Replaces CodeMirror's built-in
  //        tooltip so scrolling matches every other surface in the app.
  const acceptIndex = (index: number) => {
    const view = viewRef.current;
    const snapshot = completionRef.current;
    if (view === null || snapshot === null) return;
    completionRef.current = { ...snapshot, index };
    acceptCompletion(view);
  };

  return (
    <div ref={hostRef} className="relative h-full min-h-0 w-full overflow-hidden">
      {find !== null ? (
        <FindBar
          key={find.key}
          initialSearch={find.initial}
          replaceOpen={find.replace}
          status={findStatus}
          onQuery={changeFindQuery}
          onNext={() => {
            const view = viewRef.current;
            if (view !== null) stepFind(view, 1);
          }}
          onPrevious={() => {
            const view = viewRef.current;
            if (view !== null) stepFind(view, -1);
          }}
          onReplace={() => {
            const view = viewRef.current;
            if (view !== null) replaceCurrent(view);
          }}
          onReplaceAll={() => {
            const view = viewRef.current;
            if (view !== null) replaceAll(view);
          }}
          onClose={closeFind}
        />
      ) : null}
      {completion !== null && completion.items.length > 0 ? (
        <div
          role="listbox"
          aria-label="Suggestions"
          className="glass-modal absolute z-30 w-64 rounded-lg p-1 text-popover-foreground"
          style={{ left: completion.x, top: completion.y }}
        >
          <ScrollArea className="max-h-56">
            {completion.items.map((item, index) => (
              <div
                key={`${item.kind}:${item.label}`}
                title={item.info}
                role="option"
                aria-selected={index === completion.index}
                data-completion-index={index}
                onMouseEnter={() => {
                  const snapshot = completionRef.current;
                  if (snapshot !== null && snapshot.index !== index) {
                    const next = { ...snapshot, index };
                    completionRef.current = next;
                    setCompletion(next);
                  }
                }}
                onMouseDown={(event) => {
                  // Keep editor focus: a blur would dismiss the list before click.
                  event.preventDefault();
                }}
                onClick={() => acceptIndex(index)}
                className={cn(
                  "flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-xs",
                  index === completion.index ? "bg-selection-strong text-foreground" : "text-muted",
                )}
              >
                <Icon name={COMPLETION_ICONS[item.kind]} size={12} className="shrink-0" />
                <span className="min-w-0 flex-1 truncate font-mono">{item.label}</span>
                <span className="shrink-0 truncate text-[10px] text-muted/70">{item.detail}</span>
              </div>
            ))}
          </ScrollArea>
        </div>
      ) : null}
    </div>
  );
}
