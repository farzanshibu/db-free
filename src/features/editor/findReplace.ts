// SOT: editor-find-replace, find-matches, replace-expansion, find-highlight
import { EditorSelection, StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";

// WHAT:  Find and replace for the query editor: highlighting every match, moving
//        between them, replacing one or all.
// WHY:   Editing a long script without find-and-replace is slow and error-prone.
//        This is the editor half; the panel itself is `FindBar.tsx`, built from
//        the app's own components so it looks like everything else.
// HOW:   The query lives in a state field beside the document, so matches are
//        recomputed as the text changes and the highlight never drifts. Matching
//        is plain JavaScript regular expressions: a literal search is escaped
//        into one, whole-word wraps it in look-arounds. Replace-all is a single
//        transaction, so one undo restores everything.
// WHERE: src/features/editor/SqlEditor.tsx, src/features/editor/FindBar.tsx

export interface FindQuery {
  search: string;
  replace: string;
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
}

export interface Match {
  from: number;
  to: number;
}

export interface FindStatus {
  count: number;
  /// Index of the highlighted match, -1 when there is none.
  current: number;
  /// Why the search cannot run (an invalid regular expression), or null.
  error: string | null;
  /// More matches exist than are tracked.
  capped: boolean;
}

export const MAX_MATCHES = 10_000;

export const EMPTY_STATUS: FindStatus = { count: 0, current: -1, error: null, capped: false };

const ESCAPE = /[.*+?^${}()|[\]\\]/g;

// WHAT:  The regular expression for a query, or the reason it is not valid.
export function patternOf(query: FindQuery): { pattern: RegExp } | { error: string } {
  if (query.search.length === 0) return { error: "" };
  let source = query.regex ? query.search : query.search.replace(ESCAPE, "\\$&");
  if (query.wholeWord) source = query.regex ? `(?<![\\w$])(?:${source})(?![\\w$])` : `(?<![\\p{L}\\p{N}_$])(?:${source})(?![\\p{L}\\p{N}_$])`;
  // `u` only for literal searches: the escaping above is ours, whereas a user's own
  // pattern may carry escapes that unicode mode rejects (`\-`).
  const flags = `g${query.caseSensitive ? "" : "i"}${query.regex ? "m" : "u"}`;
  try {
    return { pattern: new RegExp(source, flags) };
  } catch (raw) {
    return { error: raw instanceof Error ? raw.message.replace(/^Invalid regular expression: /, "") : "Invalid pattern" };
  }
}

interface Found {
  hits: RegExpExecArray[];
  error: string | null;
  capped: boolean;
}

function collect(doc: string, query: FindQuery, limit: number): Found {
  const built = patternOf(query);
  if ("error" in built) return { hits: [], error: built.error.length > 0 ? built.error : null, capped: false };
  const hits: RegExpExecArray[] = [];
  const re = built.pattern;
  for (let hit = re.exec(doc); hit !== null; hit = re.exec(doc)) {
    if (hit[0].length === 0) {
      re.lastIndex += 1;
      continue;
    }
    hits.push(hit);
    if (hits.length >= limit) return { hits, error: null, capped: true };
  }
  return { hits, error: null, capped: false };
}

/// Every match of `query` in `doc`.
export function findAll(doc: string, query: FindQuery): { matches: Match[]; error: string | null; capped: boolean } {
  const found = collect(doc, query, MAX_MATCHES);
  return { matches: found.hits.map((h) => ({ from: h.index, to: h.index + h[0].length })), error: found.error, capped: found.capped };
}

// WHAT:  The replacement text for one match. A regex search expands `$1`, `$&`,
//        `$<name>` and `$$`; a literal search inserts the text as typed.
export function expandReplacement(hit: RegExpExecArray, template: string, regex: boolean): string {
  if (!regex) return template;
  return template.replace(/\$(\$|&|\d{1,2}|<[^>]+>)/g, (whole: string, token: string) => {
    if (token === "$") return "$";
    if (token === "&") return hit[0];
    if (token.startsWith("<")) return hit.groups?.[token.slice(1, -1)] ?? "";
    const group = Number(token);
    return group < hit.length ? (hit[group] ?? "") : whole;
  });
}

// ---- editor state ---------------------------------------------------------------

interface FindState {
  query: FindQuery | null;
  matches: Match[];
  current: number;
  error: string | null;
  capped: boolean;
}

const EMPTY_STATE: FindState = { query: null, matches: [], current: -1, error: null, capped: false };

/// Starts (or changes) a search; `null` ends it and clears the highlight.
export const setFind = StateEffect.define<FindQuery | null>();
/// Moves the highlight to match number `index`.
const moveFind = StateEffect.define<number>();

/// Index of the first match that starts at or after `position`, wrapping to 0.
function nearest(matches: readonly Match[], position: number): number {
  if (matches.length === 0) return -1;
  const at = matches.findIndex((m) => m.from >= position);
  return at === -1 ? 0 : at;
}

const plainMatch = Decoration.mark({ class: "cm-find-match" });
const currentMatch = Decoration.mark({ class: "cm-find-current" });

export const findField = StateField.define<FindState>({
  create: () => EMPTY_STATE,
  update(value, tr) {
    let query = value.query;
    let recompute = tr.docChanged;
    let jump: number | null = null;
    for (const effect of tr.effects) {
      if (effect.is(setFind)) {
        query = effect.value;
        recompute = true;
      } else if (effect.is(moveFind)) {
        jump = effect.value;
      }
    }
    if (!recompute && jump === null) return value;
    if (query === null) return EMPTY_STATE;
    if (!recompute) {
      return { ...value, current: value.matches.length === 0 ? -1 : Math.min(Math.max(jump ?? 0, 0), value.matches.length - 1) };
    }
    const found = findAll(tr.newDoc.toString(), query);
    return { query, matches: found.matches, current: nearest(found.matches, tr.newSelection.main.from), error: found.error, capped: found.capped };
  },
  provide: (field) =>
    EditorView.decorations.from(field, (state): DecorationSet => {
      if (state.matches.length === 0) return Decoration.none;
      return Decoration.set(state.matches.map((m, i) => (i === state.current ? currentMatch : plainMatch).range(m.from, m.to)));
    }),
});

export function findExtension(): Extension {
  return findField;
}

export function statusOf(view: EditorView): FindStatus {
  const state = view.state.field(findField, false);
  if (state === undefined) return EMPTY_STATUS;
  if (state.query === null) return EMPTY_STATUS;
  return { count: state.matches.length, current: state.current, error: state.error, capped: state.capped };
}

/// Highlights `index` and selects that match, scrolling it into view.
function select(view: EditorView, index: number): void {
  const state = view.state.field(findField, false);
  const match = state?.matches[index];
  if (match === undefined) return;
  view.dispatch({ effects: moveFind.of(index), selection: EditorSelection.range(match.from, match.to), scrollIntoView: true });
}

/// Next (1) or previous (-1) match, wrapping around.
export function stepFind(view: EditorView, direction: 1 | -1): void {
  const state = view.state.field(findField, false);
  if (state === undefined || state.matches.length === 0) return;
  const count = state.matches.length;
  select(view, (Math.max(state.current, 0) + direction + count) % count);
}

/// Selects the match at or after the caret (what Enter in the search box does first).
export function selectNearest(view: EditorView): void {
  const state = view.state.field(findField, false);
  if (state === undefined || state.matches.length === 0) return;
  select(view, nearest(state.matches, view.state.selection.main.from));
}

// WHAT:  Replaces the highlighted match and moves on to the next.
export function replaceCurrent(view: EditorView): void {
  const state = view.state.field(findField, false);
  if (state === undefined) return;
  const query = state.query;
  const match = state.matches[state.current];
  if (query === null || match === undefined) return;
  const built = patternOf(query);
  if ("error" in built) return;
  // Re-run the pattern exactly at this match to get its capture groups.
  const sticky = new RegExp(built.pattern.source, built.pattern.flags.replace("g", "") + "y");
  sticky.lastIndex = match.from;
  const hit = sticky.exec(view.state.doc.toString());
  const insert = hit === null ? query.replace : expandReplacement(hit, query.replace, query.regex);
  view.dispatch({ changes: { from: match.from, to: match.to, insert }, selection: { anchor: match.from + insert.length } });
  selectNearest(view);
}

// WHAT:  Replaces every match in one transaction (one undo). Returns how many.
export function replaceAll(view: EditorView): number {
  const query = view.state.field(findField, false)?.query;
  if (query === null || query === undefined) return 0;
  const found = collect(view.state.doc.toString(), query, Number.MAX_SAFE_INTEGER);
  if (found.hits.length === 0) return 0;
  view.dispatch({
    changes: found.hits.map((hit) => ({ from: hit.index, to: hit.index + hit[0].length, insert: expandReplacement(hit, query.replace, query.regex) })),
  });
  return found.hits.length;
}
