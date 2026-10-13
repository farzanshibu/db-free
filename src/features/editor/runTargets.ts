// SOT: run-target, run-scope, multi-cursor-run, script-offset-map
import type { RunScope, StatementSpan } from "@/lib/bindings";

// WHAT:  What a Run sends: the whole buffer, the statement the caret is in, the
//        highlighted text, or the statements under several carets / selections.
// WHY:   "Run" means different things by where the caret is and what the user
//        set in Settings → Run Without a Selection. The button has to say which,
//        and the run has to know where each piece of its script came from, so an
//        error position in the script can be underlined in the editor.
// HOW:   A target is a list of ranges of the editor's text (pieces), in the order
//        they run. The script is those ranges joined with `;` + newline, which is
//        also how an offset in the script is mapped back to the editor.
// WHERE: src/features/editor/SqlEditor.tsx (computes it), src/features/editor/QueryPane.tsx (runs it)

export interface Range {
  from: number;
  to: number;
}

export type RunKind = "all" | "statement" | "selection" | "several";

export interface RunTarget {
  kind: RunKind;
  /// Ranges of the editor's text that make up the script, in run order.
  pieces: readonly Range[];
  /// Statements the target holds: 1 for a single one, more for a selection that covers several.
  count: number;
  /// 1-based place of the statement in the buffer; 0 unless `kind` is "statement".
  index: number;
  /// Statements in the whole buffer.
  total: number;
}

const SEPARATOR = ";\n";

function clamp(range: Range, length: number): Range {
  const from = Math.min(Math.max(range.from, 0), length);
  return { from, to: Math.min(Math.max(range.to, from), length) };
}

/// The statement the caret is inside, else the next one below it, else the last.
function spanIndexAt(spans: readonly Range[], position: number): number {
  const inside = spans.findIndex((s) => position >= s.from && position <= s.to);
  if (inside !== -1) return inside;
  const below = spans.findIndex((s) => s.from > position);
  return below !== -1 ? below : spans.length - 1;
}

// WHAT:  The target for the editor's selection(s) under the run-scope setting.
// HOW:   One highlighted range runs exactly as highlighted. Several selections or
//        carets run the statements they touch, each once, in text order. A bare
//        caret runs its statement (scope "current") or everything (scope "all").
//        With no statement boundaries known yet, a run sends the whole text.
export function planRun(spans: readonly StatementSpan[], selections: readonly Range[], docLength: number, scope: RunScope): RunTarget {
  const statements = spans.map((s) => clamp({ from: s.start, to: s.end }, docLength));
  const total = statements.length;
  const whole: RunTarget = { kind: "all", pieces: [{ from: 0, to: docLength }], count: Math.max(total, 1), index: 0, total };
  const ranges = selections.map((r) => clamp(r, docLength));

  if (ranges.length === 1) {
    const only = ranges[0];
    if (only !== undefined && only.to > only.from) {
      const touched = statements.filter((s) => s.from < only.to && s.to > only.from).length;
      return { kind: "selection", pieces: [only], count: Math.max(touched, 1), index: 0, total };
    }
  }
  if (total === 0) return whole;
  if (ranges.length <= 1 && scope === "all") return whole;

  const chosen = new Set<number>();
  for (const range of ranges) {
    if (range.to > range.from) {
      statements.forEach((s, i) => {
        if (s.from < range.to && s.to > range.from) chosen.add(i);
      });
    } else {
      chosen.add(spanIndexAt(statements, range.from));
    }
  }
  const picked = [...chosen].sort((a, b) => a - b);
  const pieces = picked.flatMap((i) => statements[i] ?? []);
  if (pieces.length === 0) return whole;
  const first = picked[0] ?? 0;
  return pieces.length === 1
    ? { kind: "statement", pieces, count: 1, index: first + 1, total }
    : { kind: "several", pieces, count: pieces.length, index: 0, total };
}

/// The script a target sends: its ranges of `doc`, joined.
export function joinScript(doc: string, pieces: readonly Range[]): string {
  return pieces.map((p) => doc.slice(p.from, p.to)).join(SEPARATOR);
}

// WHAT:  Where an offset in the script sits in the editor.
// WHY:   The database reports a position inside the script it was sent; the
//        squiggle has to go under the same text in the editor.
export function scriptOffsetToDoc(pieces: readonly Range[], offset: number): number {
  let start = 0;
  for (const piece of pieces) {
    const length = piece.to - piece.from;
    if (offset <= start + length) return piece.from + Math.max(offset - start, 0);
    start += length + SEPARATOR.length;
  }
  const last = pieces[pieces.length - 1];
  return last === undefined ? 0 : last.to;
}

/// The label the Run button wears for a target.
export function runLabel(target: RunTarget | null): string {
  if (target === null) return "Run";
  switch (target.kind) {
    case "selection":
      return target.count > 1 ? `Run selection (${target.count})` : "Run selection";
    case "statement":
      return target.total > 1 ? `Run statement ${target.index}` : "Run";
    case "several":
      return `Run ${target.count} queries`;
    case "all":
      return target.total > 1 ? `Run all ${target.total}` : "Run";
  }
}

export function sameTarget(a: RunTarget | null, b: RunTarget): boolean {
  return (
    a !== null &&
    a.kind === b.kind &&
    a.count === b.count &&
    a.index === b.index &&
    a.total === b.total &&
    a.pieces.length === b.pieces.length &&
    a.pieces.every((p, i) => {
      const other = b.pieces[i];
      return other?.from === p.from && other.to === p.to;
    })
  );
}
