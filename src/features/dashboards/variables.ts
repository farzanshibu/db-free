// SOT: dashboard-variables, variable-substitution, variable-sql-literals, variable-defaults, variable-types, variable-completions
import type { DashboardVariable, Family } from "@/lib/bindings";
import { paramLiteral } from "@/lib/params";

// WHAT:  Dashboard variables turned into SQL: `{{name}}`, `{{range.start}}` and
//        `{{range.end}}` in a widget's statement become literals of the right
//        type, quoted and escaped for the connection's dialect.
// WHY:   Variables used to be pasted into the statement as typed, so a value
//        like `x' OR '1'='1` rewrote the query. Every value is data: it is
//        written as one literal, never as SQL, and a token that sits where a
//        literal cannot go (a comment, an identifier) is refused instead of
//        substituted.
// HOW:   One pass over the statement that tracks what the scanner is inside
//        (code, a '...' string, a quoted identifier, a comment, a dollar-quoted
//        body). In code a token becomes a full literal (`'eu'`, `42`, `NULL`,
//        `'a','b'`, `'2026-01-01 00:00:00' AND '2026-01-31 23:59:59'`); inside a
//        string it becomes the escaped text alone, so the older `'{{name}}'`
//        spelling keeps working. The caller compares the resulting statement to
//        decide which tiles to re-run.
// WHERE: src/features/dashboards/DashboardTab.tsx, src/lib/params.ts (quoting),
//        src-tauri/src/model/documents.rs (DashboardVariable)

// ---------------------------------------------------------------- dialects
/// What differs between engines when a value becomes a literal.
export interface SqlDialect {
  /// A backslash starts an escape inside '...' (MySQL, ClickHouse, BigQuery, Snowflake).
  backslashEscapes: boolean;
  /// There are no TRUE / FALSE literals: booleans are 1 / 0 (MySQL, SQLite, SQL Server, Oracle).
  numericBooleans: boolean;
  /// `#` starts a line comment (MySQL).
  hashComments: boolean;
  /// How a day becomes a timestamp literal.
  dates: "iso" | "sqlite" | "mssql" | "oracle";
}

const SQLITE_LIKE: readonly Family[] = ["sqlite", "libsql", "val_town", "cloudflare_d1"];
const BACKSLASH_FAMILIES: readonly Family[] = ["mysql", "clickhouse", "bigquery", "snowflake"];
const NUMERIC_BOOLEAN_FAMILIES: readonly Family[] = ["mysql", "mssql", "oracle", ...SQLITE_LIKE];

export function dialectOfFamily(family: Family): SqlDialect {
  return {
    backslashEscapes: BACKSLASH_FAMILIES.includes(family),
    numericBooleans: NUMERIC_BOOLEAN_FAMILIES.includes(family),
    hashComments: family === "mysql",
    dates: SQLITE_LIKE.includes(family) ? "sqlite" : family === "mssql" ? "mssql" : family === "oracle" ? "oracle" : "iso",
  };
}

// ---------------------------------------------------------------- variable model
export type VariableType = DashboardVariable["type"];

export const VARIABLE_TYPES: readonly { value: VariableType; label: string }[] = [
  { value: "date_range", label: "Date range" },
  { value: "text", label: "Text" },
  { value: "number", label: "Number" },
  { value: "select", label: "Select" },
  { value: "multi_select", label: "Multi-select" },
  { value: "boolean", label: "Boolean" },
];

/// The part of a name a token may use; also what the name field keeps.
const NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function isValidVariableName(name: string): boolean {
  return NAME_PATTERN.test(name);
}

/// Any text reduced to what a token can spell: other characters become `_`,
/// and a leading digit gets an underscore in front.
export function sanitizeVariableName(raw: string): string {
  const cleaned = raw.replace(/[^A-Za-z0-9_]/g, "_");
  return /^[0-9]/.test(cleaned) ? `_${cleaned}` : cleaned;
}

export function variableName(variable: DashboardVariable): string {
  return variable.name;
}

/// `var_1`, `var_2`... the first name no variable uses.
export function nextVariableName(variables: readonly DashboardVariable[]): string {
  let n = variables.length + 1;
  while (variables.some((v) => v.name === `var_${n}`)) n += 1;
  return `var_${n}`;
}

/// A new variable of a type with an empty value.
export function newVariable(type: VariableType, name: string): DashboardVariable {
  switch (type) {
    case "date_range":
      return { type, name, start: "", end: "" };
    case "text":
      return { type, name, value: "" };
    case "number":
      return { type, name, value: null };
    case "select":
      return { type, name, value: "", options: [] };
    case "multi_select":
      return { type, name, values: [], options: [] };
    case "boolean":
      return { type, name, value: false };
  }
}

/// The same variable under another type: the name stays, the value starts over.
export function retypeVariable(variable: DashboardVariable, type: VariableType): DashboardVariable {
  const next = newVariable(type, variable.name);
  const options = variable.type === "select" || variable.type === "multi_select" ? variable.options : null;
  if (options !== null && (next.type === "select" || next.type === "multi_select")) return { ...next, options };
  return next;
}

/// The variable with its options changed; a value that is no longer an option is dropped.
export function withOptions(variable: DashboardVariable, options: readonly string[]): DashboardVariable {
  if (variable.type === "select") {
    return { ...variable, options: [...options], value: options.includes(variable.value) ? variable.value : "" };
  }
  if (variable.type === "multi_select") {
    return { ...variable, options: [...options], values: variable.values.filter((v) => options.includes(v)) };
  }
  return variable;
}

/// One line saying what the variable currently holds, for the toolbar button and tooltips.
export function describeVariable(variable: DashboardVariable): string {
  switch (variable.type) {
    case "date_range":
      return variable.start.length === 0 && variable.end.length === 0 ? "any date" : `${variable.start || "…"} to ${variable.end || "…"}`;
    case "text":
    case "select":
      return variable.value.length === 0 ? "empty" : variable.value;
    case "number":
      return variable.value === null ? "empty" : String(variable.value);
    case "multi_select":
      return variable.values.length === 0 ? "none" : variable.values.length === variable.options.length ? "all" : `${variable.values.length} selected`;
    case "boolean":
      return variable.value ? "true" : "false";
  }
}

/// Every token a widget's SQL may use, for completion: `name`, plus `name.start`
/// and `name.end` for a date range. `detail` says what the token inserts.
export function variableTokens(variables: readonly DashboardVariable[]): { token: string; detail: string }[] {
  return variables.flatMap((v) =>
    v.type === "date_range"
      ? [
          { token: v.name, detail: "start AND end" },
          { token: `${v.name}.start`, detail: "range start" },
          { token: `${v.name}.end`, detail: "range end" },
        ]
      : [{ token: v.name, detail: VARIABLE_TYPES.find((t) => t.value === v.type)?.label ?? v.type }],
  );
}

// ---------------------------------------------------------------- literals
function stripNul(raw: string): string {
  return raw.replace(/\u0000/g, "");
}

/// `'text'` with quotes doubled (and backslashes, where they escape).
function textLiteral(dialect: SqlDialect, raw: string): string {
  const safe = stripNul(raw);
  return paramLiteral({ value: dialect.backslashEscapes ? safe.replace(/\\/g, "\\\\") : safe, mode: "text" });
}

/// The same text as it reads between quotes someone else already wrote.
function innerText(dialect: SqlDialect, raw: string): string {
  return textLiteral(dialect, raw).slice(1, -1);
}

function numberText(value: number | null): string | null {
  return value !== null && Number.isFinite(value) ? String(value) : null;
}

function booleanText(dialect: SqlDialect, value: boolean): string {
  if (dialect.numericBooleans) return value ? "1" : "0";
  return value ? "TRUE" : "FALSE";
}

interface Instant {
  date: string;
  time: string;
}

const DAY = /^(\d{4})-(\d{2})-(\d{2})$/;
const DAY_TIME = /^(\d{4}-\d{2}-\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?$/;

function isRealDay(text: string): boolean {
  const parts = DAY.exec(text);
  if (parts === null) return false;
  const [year, month, day] = [Number(parts[1]), Number(parts[2]), Number(parts[3])];
  const probe = new Date(Date.UTC(year, month - 1, day));
  return probe.getUTCFullYear() === year && probe.getUTCMonth() === month - 1 && probe.getUTCDate() === day;
}

/// A calendar day, or a day with a time, read from the text a range holds.
/// Anything else is null (the caller quotes it as typed).
export function parseInstant(text: string): { date: string; time: string | null } | null {
  const trimmed = text.trim();
  if (isRealDay(trimmed)) return { date: trimmed, time: null };
  const parts = DAY_TIME.exec(trimmed);
  if (parts === null || !isRealDay(parts[1] ?? "")) return null;
  const [hour, minute, second] = [Number(parts[2]), Number(parts[3]), Number(parts[4] ?? "0")];
  if (hour > 23 || minute > 59 || second > 59) return null;
  const two = (n: number) => String(n).padStart(2, "0");
  return { date: parts[1] ?? "", time: `${two(hour)}:${two(minute)}:${two(second)}` };
}

/// The moment a bound means: a bare day starts at 00:00:00 and ends at 23:59:59,
/// so a range of days covers the last day whole.
function instantOf(text: string, edge: "start" | "end"): Instant | null {
  const parsed = parseInstant(text);
  if (parsed === null) return null;
  return { date: parsed.date, time: parsed.time ?? (edge === "start" ? "00:00:00" : "23:59:59") };
}

function dateLiteral(dialect: SqlDialect, text: string, edge: "start" | "end"): string {
  if (text.trim().length === 0) return "NULL";
  const at = instantOf(text, edge);
  if (at === null) return textLiteral(dialect, text);
  switch (dialect.dates) {
    case "sqlite": {
      // Text comparison: a bare start day sorts before every time on that day
      // and before a date-only column's own value, so it stays bare.
      const bare = edge === "start" && parseInstant(text)?.time === null;
      return `'${bare ? at.date : `${at.date} ${at.time}`}'`;
    }
    case "mssql":
      return `'${at.date}T${at.time}'`;
    case "oracle":
      return `TIMESTAMP '${at.date} ${at.time}'`;
    case "iso":
      return `'${at.date} ${at.time}'`;
  }
}

/// A date bound as the bare text between quotes someone else wrote.
function dateInnerText(dialect: SqlDialect, text: string, edge: "start" | "end"): string {
  const at = instantOf(text, edge);
  if (at === null) return innerText(dialect, text);
  return dialect.dates === "mssql" ? `${at.date}T${at.time}` : `${at.date} ${at.time}`;
}

type Bound = "start" | "end" | null;

/// The literal a token becomes in code position, or null when the variable
/// cannot stand there (`.start` on something that is not a date range).
function codeLiteral(dialect: SqlDialect, variable: DashboardVariable, bound: Bound): string | null {
  if (bound !== null) {
    if (variable.type !== "date_range") return null;
    return dateLiteral(dialect, bound === "start" ? variable.start : variable.end, bound);
  }
  switch (variable.type) {
    case "date_range":
      return `${dateLiteral(dialect, variable.start, "start")} AND ${dateLiteral(dialect, variable.end, "end")}`;
    case "text":
    case "select":
      return variable.value.length === 0 ? "NULL" : textLiteral(dialect, variable.value);
    case "number":
      return numberText(variable.value) ?? "NULL";
    case "multi_select":
      return variable.values.length === 0 ? "NULL" : variable.values.map((v) => textLiteral(dialect, v)).join(",");
    case "boolean":
      return booleanText(dialect, variable.value);
  }
}

/// The text a token becomes between someone's own quotes, or null when it cannot.
function stringText(dialect: SqlDialect, variable: DashboardVariable, bound: Bound): string | null {
  if (bound !== null) {
    if (variable.type !== "date_range") return null;
    const text = bound === "start" ? variable.start : variable.end;
    return text.trim().length === 0 ? "" : dateInnerText(dialect, text, bound);
  }
  switch (variable.type) {
    case "date_range":
      return null;
    case "text":
    case "select":
      return innerText(dialect, variable.value);
    case "number":
      return numberText(variable.value) ?? "";
    case "multi_select":
      return variable.values.map((v) => innerText(dialect, v)).join(",");
    case "boolean":
      return dialect.numericBooleans ? (variable.value ? "1" : "0") : variable.value ? "true" : "false";
  }
}

// ---------------------------------------------------------------- scanning
const TOKEN = /^\{\{\s*([A-Za-z_][A-Za-z0-9_]*)(?:\.(start|end))?\s*\}\}/;
const DOLLAR_TAG = /^\$[A-Za-z_]*\$/;

export interface Substitution {
  sql: string;
  /// Tokens naming no variable (spelt as written, e.g. `region`).
  missing: string[];
  /// Tokens in a position that takes no literal, or `.start` / `.end` on something
  /// that is not a date range (spelt as written, e.g. `period`).
  misplaced: string[];
}

function pushOnce(list: string[], name: string): void {
  if (!list.includes(name)) list.push(name);
}

/// Where a quoted span that opens at `start` ends: just past its closing quote
/// (`closed`), or the end of the text when it never closes. A doubled quote is
/// part of the span, and so is a backslash escape where the dialect has them.
function quotedEnd(sql: string, start: number, quote: string, backslash: boolean): { end: number; closed: boolean } {
  let i = start + 1;
  while (i < sql.length) {
    const ch = sql.charAt(i);
    if (backslash && ch === "\\" && quote === "'") {
      i += 2;
      continue;
    }
    if (ch === quote) {
      if (sql.charAt(i + 1) === quote) {
        i += 2;
        continue;
      }
      return { end: i + 1, closed: true };
    }
    i += 1;
  }
  return { end: sql.length, closed: false };
}

// WHAT:  The statement with every variable token replaced.
// WHY:   See the file header: values are data, tokens in comments and
//        identifiers are refused, the rest is escaped for the dialect.
export function substituteVariables(sql: string, variables: readonly DashboardVariable[], dialect: SqlDialect): Substitution {
  const byName = new Map(variables.map((v) => [v.name, v]));
  const missing: string[] = [];
  const misplaced: string[] = [];
  let out = "";
  let i = 0;

  // A token at `i`, replaced by `render`'s answer (or kept as written and
  // reported when the name is undefined or the position cannot take it).
  const token = (render: (variable: DashboardVariable, bound: Bound) => string | null): boolean => {
    const hit = TOKEN.exec(sql.slice(i, i + 80));
    if (hit === null) return false;
    const name = hit[1] ?? "";
    const bound: Bound = hit[2] === "start" ? "start" : hit[2] === "end" ? "end" : null;
    const variable = byName.get(name);
    if (variable === undefined) {
      pushOnce(missing, name);
      out += hit[0];
    } else {
      const text = render(variable, bound);
      if (text === null) {
        pushOnce(misplaced, name);
        out += hit[0];
      } else {
        out += text;
      }
    }
    i += hit[0].length;
    return true;
  };

  // A span whose contents are copied untouched; tokens inside it that name a
  // variable are reported, since they cannot be filled in there.
  const verbatim = (to: number, report: boolean): void => {
    const span = sql.slice(i, to);
    if (report) {
      for (const hit of span.matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_]*)(?:\.(?:start|end))?\s*\}\}/g)) {
        const name = hit[1] ?? "";
        if (byName.has(name)) pushOnce(misplaced, name);
      }
    }
    out += span;
    i = to;
  };

  while (i < sql.length) {
    const ch = sql.charAt(i);
    const next = sql.charAt(i + 1);
    if (ch === "'") {
      // Inside a string: tokens become escaped text, the string's own quotes stay.
      const span = quotedEnd(sql, i, "'", dialect.backslashEscapes);
      const stop = span.closed ? span.end - 1 : span.end;
      out += "'";
      i += 1;
      while (i < stop) {
        if (sql.charAt(i) === "{" && sql.charAt(i + 1) === "{" && token((v, b) => stringText(dialect, v, b))) continue;
        out += sql.charAt(i);
        i += 1;
      }
      out += sql.slice(i, span.end);
      i = span.end;
      continue;
    }
    if (ch === '"' || ch === "`") {
      verbatim(quotedEnd(sql, i, ch, false).end, true);
      continue;
    }
    if ((ch === "-" && next === "-") || (dialect.hashComments && ch === "#")) {
      const eol = sql.indexOf("\n", i);
      verbatim(eol < 0 ? sql.length : eol, false);
      continue;
    }
    if (ch === "/" && next === "*") {
      const close = sql.indexOf("*/", i + 2);
      verbatim(close < 0 ? sql.length : close + 2, false);
      continue;
    }
    if (ch === "$") {
      const tag = DOLLAR_TAG.exec(sql.slice(i, i + 40));
      if (tag !== null) {
        const close = sql.indexOf(tag[0], i + tag[0].length);
        verbatim(close < 0 ? sql.length : close + tag[0].length, true);
        continue;
      }
    }
    if (ch === "{" && next === "{" && token((v, b) => codeLiteral(dialect, v, b))) continue;
    out += ch;
    i += 1;
  }
  return { sql: out, missing, misplaced };
}

// WHAT:  Why a statement cannot run yet, in one sentence, or null when it can.
export function substitutionProblem(result: Substitution): string | null {
  if (result.missing.length > 0) {
    return `Unknown variable${result.missing.length > 1 ? "s" : ""}: ${result.missing.map((n) => `{{${n}}}`).join(", ")}. Add ${result.missing.length > 1 ? "them" : "it"} under Variables.`;
  }
  if (result.misplaced.length > 0) {
    return `${result.misplaced.map((n) => `{{${n}}}`).join(", ")} cannot be used there: a date range takes .start / .end inside quotes, and no variable fits a comment or a quoted name.`;
  }
  return null;
}
