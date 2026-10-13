// SOT: postgres-array-literal, array-chip-parsing

// WHAT:  Postgres array text (`{a,"b c",NULL}`) taken apart and put back together.
// WHY:   The adapter hands arrays over as that text, and the same text is what a
//        write has to send, so the chip editor edits a list and converts at the edge.
// HOW:   One dimension only: a nested array returns null and the cell stays a text
//        edit. NULL elements are `null` in the list.
// WHERE: src/components/global/ValueEditor.tsx (ArrayEditor)
export function parsePgArray(text: string): (string | null)[] | null {
  const source = text.trim();
  if (!source.startsWith("{") || !source.endsWith("}")) return null;
  const body = source.slice(1, -1);
  const items: (string | null)[] = [];
  let i = 0;
  if (body.trim().length === 0) return items;
  while (i <= body.length) {
    while (i < body.length && body[i] === " ") i += 1;
    if (body[i] === "{") return null;
    if (body[i] === '"') {
      let out = "";
      i += 1;
      for (;;) {
        const ch = body[i];
        if (ch === undefined) return null;
        if (ch === "\\") {
          out += body[i + 1] ?? "";
          i += 2;
        } else if (ch === '"') {
          i += 1;
          break;
        } else {
          out += ch;
          i += 1;
        }
      }
      items.push(out);
    } else {
      const end = body.indexOf(",", i);
      const raw = body.slice(i, end === -1 ? body.length : end).trim();
      items.push(raw.toUpperCase() === "NULL" ? null : raw);
      i = end === -1 ? body.length : end;
    }
    while (i < body.length && body[i] === " ") i += 1;
    if (i >= body.length) break;
    if (body[i] !== ",") return null;
    i += 1;
  }
  return items;
}

/// A list as Postgres array text; anything that is not plain gets quoted.
export function formatPgArray(items: readonly (string | null)[]): string {
  const part = (item: string | null): string => {
    if (item === null) return "NULL";
    if (item.length === 0 || /[\s{},"\\]/.test(item) || item.toUpperCase() === "NULL") {
      return `"${item.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
    }
    return item;
  };
  return `{${items.map(part).join(",")}}`;
}
