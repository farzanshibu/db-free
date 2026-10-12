// SOT: line-diff, save-diff-preview

// WHAT:  A line-by-line diff of two texts, for the "overwrite the saved query?"
//        confirmation.
// WHY:   Overwriting a saved query with the tab's text is only safe to approve if
//        you can see what changes: which lines go and which come.
// HOW:   The common first and last lines are peeled off, then a longest-common-
//        subsequence table over what is left. A very large middle (both sides
//        over a million cell pairs) skips the table and reports the middle as one
//        removal and one addition, which is correct, just coarser.
// WHERE: src/features/editor/SaveQueryDialog.tsx

export type DiffKind = "same" | "add" | "del";

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

const MAX_CELLS = 1_000_000;

export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head += 1;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail += 1;
  const midA = a.slice(head, a.length - tail);
  const midB = b.slice(head, b.length - tail);

  const out: DiffLine[] = a.slice(0, head).map((text) => ({ kind: "same", text }));
  out.push(...(midA.length * midB.length > MAX_CELLS ? coarse(midA, midB) : lcsDiff(midA, midB)));
  out.push(...a.slice(a.length - tail).map((text): DiffLine => ({ kind: "same", text })));
  return out;
}

function coarse(a: readonly string[], b: readonly string[]): DiffLine[] {
  return [...a.map((text): DiffLine => ({ kind: "del", text })), ...b.map((text): DiffLine => ({ kind: "add", text }))];
}

function lcsDiff(a: readonly string[], b: readonly string[]): DiffLine[] {
  const rows = a.length;
  const cols = b.length;
  // table[i][j] = length of the longest common subsequence of a[i..] and b[j..].
  const table: number[][] = Array.from({ length: rows + 1 }, () => new Array<number>(cols + 1).fill(0));
  for (let i = rows - 1; i >= 0; i -= 1) {
    const row = table[i];
    const below = table[i + 1];
    if (row === undefined || below === undefined) continue;
    for (let j = cols - 1; j >= 0; j -= 1) {
      row[j] = a[i] === b[j] ? (below[j + 1] ?? 0) + 1 : Math.max(below[j] ?? 0, row[j + 1] ?? 0);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < rows || j < cols) {
    const left = a[i];
    const right = b[j];
    if (left !== undefined && right !== undefined && left === right) {
      out.push({ kind: "same", text: left });
      i += 1;
      j += 1;
    } else if (left !== undefined && (right === undefined || (table[i + 1]?.[j] ?? 0) >= (table[i]?.[j + 1] ?? 0))) {
      out.push({ kind: "del", text: left });
      i += 1;
    } else if (right !== undefined) {
      out.push({ kind: "add", text: right });
      j += 1;
    }
  }
  return out;
}

/// How many lines the change adds and removes.
export function diffStat(lines: readonly DiffLine[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of lines) {
    if (line.kind === "add") added += 1;
    else if (line.kind === "del") removed += 1;
  }
  return { added, removed };
}
