// SOT: query-parameters, param-detection, param-binding, sql-literal-quoting

// WHAT:  Named placeholders in an editor script — `:name` (SQL only) and
//        `{{name}}` (any language) — found, and replaced by literals before Run.
// WHY:   Re-running a query with a different id or date should not mean
//        hunting through the text for every place it appears. Dashboards
//        already substitute `{{var}}`; the editor speaks the same syntax plus
//        the `:name` form SQL clients use.
// HOW:   A small scanner skips string literals, quoted identifiers, comments
//        and dollar-quoted bodies, so `'12:30'`, `"a:b"` and `-- :x` are left
//        alone. `:name` only counts after a non-identifier character, so
//        Postgres casts (`::int`), assignments (`:=`), Snowflake paths
//        (`src:customer`) and array slices (`a[1:2]`) are not parameters.
//        Values are substituted client-side as literals: the statement the
//        block classifies, logs and runs is the one the user would have typed.
// WHERE: src/features/editor/QueryPane.tsx (prompt before Run)

export type ParamMode = "auto" | "text" | "raw";

export interface ParamValue {
  value: string;
  mode: ParamMode;
}

interface Hit {
  name: string;
  start: number;
  end: number;
}

const IDENT_START = /[A-Za-z_]/;
const IDENT = /[A-Za-z0-9_]/;

function scan(sql: string, colonParams: boolean): Hit[] {
  const hits: Hit[] = [];
  let i = 0;
  while (i < sql.length) {
    const ch = sql.charAt(i);
    const next = sql.charAt(i + 1);
    if (ch === "'" || ch === '"' || ch === "`") {
      i = skipQuoted(sql, i, ch);
      continue;
    }
    if (ch === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      i = end < 0 ? sql.length : end + 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? sql.length : end + 2;
      continue;
    }
    if (ch === "$") {
      const tag = /^\$[A-Za-z_]*\$/.exec(sql.slice(i));
      if (tag) {
        const close = sql.indexOf(tag[0], i + tag[0].length);
        i = close < 0 ? sql.length : close + tag[0].length;
        continue;
      }
    }
    if (ch === "{" && next === "{") {
      const close = sql.indexOf("}}", i + 2);
      const name = close < 0 ? "" : sql.slice(i + 2, close).trim();
      if (close >= 0 && /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
        hits.push({ name, start: i, end: close + 2 });
        i = close + 2;
        continue;
      }
    }
    if (colonParams && ch === ":" && IDENT_START.test(next)) {
      const before = i === 0 ? "" : sql.charAt(i - 1);
      if (before !== ":" && !IDENT.test(before) && before !== "]" && before !== ")") {
        let end = i + 1;
        while (end < sql.length && IDENT.test(sql.charAt(end))) end += 1;
        if (sql.charAt(end) !== "=") {
          hits.push({ name: sql.slice(i + 1, end), start: i, end });
          i = end;
          continue;
        }
      }
    }
    i += 1;
  }
  return hits;
}

/// Index just past the closing quote; a doubled quote is an escaped one.
function skipQuoted(sql: string, start: number, quote: string): number {
  let i = start + 1;
  while (i < sql.length) {
    const ch = sql.charAt(i);
    if (ch === "\\" && quote === "'") {
      i += 2;
      continue;
    }
    if (ch === quote) {
      if (sql.charAt(i + 1) === quote) {
        i += 2;
        continue;
      }
      return i + 1;
    }
    i += 1;
  }
  return sql.length;
}

/// Distinct parameter names in order of first appearance.
export function findParams(sql: string, colonParams: boolean): string[] {
  const seen: string[] = [];
  for (const hit of scan(sql, colonParams)) if (!seen.includes(hit.name)) seen.push(hit.name);
  return seen;
}

/// The literal a value becomes. Auto: numbers, booleans and NULL stay bare,
/// everything else is a quoted string.
export function paramLiteral(param: ParamValue): string {
  const raw = param.value;
  if (param.mode === "raw") return raw;
  if (param.mode === "auto") {
    const trimmed = raw.trim();
    if (/^-?\d+(\.\d+)?([eE][-+]?\d+)?$/.test(trimmed)) return trimmed;
    if (/^(null|true|false)$/i.test(trimmed)) return trimmed.toUpperCase();
  }
  return `'${raw.replace(/'/g, "''")}'`;
}

export function bindParams(sql: string, colonParams: boolean, values: Readonly<Record<string, ParamValue>>): string {
  let out = "";
  let at = 0;
  for (const hit of scan(sql, colonParams)) {
    const param = values[hit.name];
    if (param === undefined) continue;
    out += sql.slice(at, hit.start) + paramLiteral(param);
    at = hit.end;
  }
  return out + sql.slice(at);
}
