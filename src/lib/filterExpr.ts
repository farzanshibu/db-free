// SOT: filter-expression, typed-filter-bar, filter-parser, filter-completion, filter-formatter
import type { FilterMode, FilterOp, FilterRule } from "./bindings";

// WHAT:  The typed filter bar's language: `status = active and revenue > 100`.
//        Text in, filter rules out (and rules back to text), with the spans of
//        anything wrong and completions for the word under the cursor.
// WHY:   A row of dropdowns is slow for someone who knows what they want to ask;
//        the same rules the popover edits can be typed. The grid still sends
//        FilterRules, so every engine's adapter honours them as before.
// HOW:   rule := column operator [value]; rules join with `and` or `or` (one kind
//        per expression — the table has a single All / Any mode). Columns bare or
//        "double-quoted" / `backticked`; values bare, 'single-quoted' ('' for a
//        quote) or a list `(a, b)` after `in`. `x = null` means `x is null`.
// WHERE: src/features/grid/FilterBar.tsx, src-tauri/src/model/query.rs (FilterOp)

export interface FilterColumn {
  name: string;
  typeName?: string | undefined;
}

export interface ExprError {
  message: string;
  /// Character offsets into the typed text.
  start: number;
  end: number;
}

export type ParseResult = { ok: true; rules: FilterRule[]; mode: FilterMode } | { ok: false; errors: ExprError[] };

/// How each operator is typed. Word operators are matched case-insensitively.
export const OP_TEXT = {
  eq: "=",
  ne: "!=",
  gt: ">",
  gte: ">=",
  lt: "<",
  lte: "<=",
  contains: "contains",
  not_contains: "not contains",
  starts_with: "starts with",
  ends_with: "ends with",
  in: "in",
  is_null: "is null",
  is_not_null: "is not null",
} satisfies Record<FilterOp, string>;

const SYMBOL_OPS: Readonly<Record<string, FilterOp>> = { "=": "eq", "==": "eq", "!=": "ne", "<>": "ne", ">": "gt", ">=": "gte", "<": "lt", "<=": "lte" };
const NEEDS_VALUE: Readonly<Record<FilterOp, boolean>> = {
  eq: true,
  ne: true,
  gt: true,
  gte: true,
  lt: true,
  lte: true,
  contains: true,
  not_contains: true,
  starts_with: true,
  ends_with: true,
  in: true,
  is_null: false,
  is_not_null: false,
};
const KEYWORDS: ReadonlySet<string> = new Set(["and", "or", "not", "in", "is", "null", "contains", "starts", "ends", "with", "like"]);

type TokenKind = "word" | "string" | "ident" | "symbol";

interface Token {
  kind: TokenKind;
  /// Decoded text: quotes removed, doubled quotes collapsed.
  text: string;
  start: number;
  end: number;
}

const SYMBOLS = ["<=", ">=", "<>", "!=", "==", "=", "<", ">", ",", "(", ")"];
const WORD_STOP = /[\s=<>!(),'"`]/;

function tokenize(source: string): { tokens: Token[]; error: ExprError | null } {
  const tokens: Token[] = [];
  let i = 0;
  while (i < source.length) {
    const ch = source[i] ?? "";
    if (/\s/.test(ch)) {
      i += 1;
      continue;
    }
    if (ch === "'" || ch === '"' || ch === "`") {
      const start = i;
      let text = "";
      i += 1;
      for (;;) {
        const c = source[i];
        if (c === undefined) return { tokens, error: { message: "This quote is never closed.", start, end: source.length } };
        if (c === ch) {
          if (source[i + 1] === ch) {
            text += ch;
            i += 2;
            continue;
          }
          i += 1;
          break;
        }
        text += c;
        i += 1;
      }
      tokens.push({ kind: ch === "'" ? "string" : "ident", text, start, end: i });
      continue;
    }
    const symbol = SYMBOLS.find((s) => source.startsWith(s, i));
    if (symbol !== undefined) {
      tokens.push({ kind: "symbol", text: symbol, start: i, end: i + symbol.length });
      i += symbol.length;
      continue;
    }
    const start = i;
    while (i < source.length && !WORD_STOP.test(source[i] ?? "")) i += 1;
    if (i === start) {
      // A character no rule knows (a lone `!`): report it rather than loop.
      return { tokens, error: { message: `Unexpected "${ch}".`, start, end: start + 1 } };
    }
    tokens.push({ kind: "word", text: source.slice(start, i), start, end: i });
  }
  return { tokens, error: null };
}

const wordIs = (token: Token | undefined, word: string): boolean => token?.kind === "word" && token.text.toLowerCase() === word;

/// Resolves what the user typed to a real column: exact, then ignoring case.
function resolveColumn(name: string, columns: readonly FilterColumn[]): string | null {
  const exact = columns.find((c) => c.name === name);
  if (exact) return exact.name;
  const lower = name.toLowerCase();
  return columns.find((c) => c.name.toLowerCase() === lower)?.name ?? null;
}

interface OperatorRead {
  op: FilterOp | null;
  /// Tokens consumed, including a partial operator word.
  used: number;
  /// What would complete a half-typed operator (`not` → `contains`).
  expects: string[];
  error: ExprError | null;
}

// WHAT:  Reads the operator at `at`: a symbol or a (multi-)word operator.
function readOperator(tokens: readonly Token[], at: number, end: number): OperatorRead {
  const first = tokens[at];
  const none: OperatorRead = { op: null, used: 0, expects: [], error: null };
  if (first === undefined) return none;
  if (first.kind === "symbol") {
    const op = SYMBOL_OPS[first.text];
    if (op !== undefined) return { ...none, op, used: 1 };
    return { ...none, error: { message: `"${first.text}" is not an operator.`, start: first.start, end: first.end } };
  }
  if (first.kind !== "word") return { ...none, error: { message: "Expected an operator such as = or contains.", start: first.start, end: first.end } };
  const word = first.text.toLowerCase();
  const second = tokens[at + 1];
  const third = tokens[at + 2];
  switch (word) {
    case "contains":
      return { ...none, op: "contains", used: 1 };
    case "not":
      if (second === undefined) return { ...none, used: 1, expects: ["contains"] };
      if (wordIs(second, "contains")) return { ...none, op: "not_contains", used: 2 };
      return { ...none, error: { message: 'Only "not contains" can follow "not" here.', start: first.start, end: second.end } };
    case "starts":
    case "startswith":
      if (word === "startswith") return { ...none, op: "starts_with", used: 1 };
      if (second === undefined) return { ...none, used: 1, expects: ["with"] };
      if (wordIs(second, "with")) return { ...none, op: "starts_with", used: 2 };
      return { ...none, error: { message: 'Write "starts with".', start: first.start, end: second.end } };
    case "ends":
    case "endswith":
      if (word === "endswith") return { ...none, op: "ends_with", used: 1 };
      if (second === undefined) return { ...none, used: 1, expects: ["with"] };
      if (wordIs(second, "with")) return { ...none, op: "ends_with", used: 2 };
      return { ...none, error: { message: 'Write "ends with".', start: first.start, end: second.end } };
    case "in":
      return { ...none, op: "in", used: 1 };
    case "is":
      if (second === undefined) return { ...none, used: 1, expects: ["null", "not null"] };
      if (wordIs(second, "null")) return { ...none, op: "is_null", used: 2 };
      if (wordIs(second, "not")) {
        if (third === undefined) return { ...none, used: 2, expects: ["null"] };
        if (wordIs(third, "null")) return { ...none, op: "is_not_null", used: 3 };
      }
      return { ...none, error: { message: 'Write "is null" or "is not null".', start: first.start, end: (third ?? second).end } };
    default:
      return { ...none, error: { message: `"${first.text}" is not an operator. Try =, !=, >, contains, starts with, in, is null.`, start: first.start, end } };
  }
}

interface ValueRead {
  value: string | null;
  /// The tokens that made it up.
  used: number;
  /// Only the list opened (`in (a,`), so more is expected.
  open: boolean;
  error: ExprError | null;
  /// A bare `null` was written: the caller turns `=`/`!=` into is_null / is_not_null.
  bareNull: boolean;
}

function isValueToken(token: Token): boolean {
  return token.kind !== "symbol";
}

// WHAT:  Reads a value: one token, or for `in` a comma-separated list, with or without parentheses.
function readValue(tokens: readonly Token[], at: number, op: FilterOp): ValueRead {
  const empty: ValueRead = { value: null, used: 0, open: false, error: null, bareNull: false };
  const first = tokens[at];
  if (first === undefined) return empty;
  if (op !== "in") {
    if (!isValueToken(first)) return { ...empty, error: { message: "Expected a value.", start: first.start, end: first.end } };
    return { ...empty, value: first.text, used: 1, bareNull: first.kind === "word" && first.text.toLowerCase() === "null" };
  }
  let i = at;
  const parenthesised = first.kind === "symbol" && first.text === "(";
  if (parenthesised) i += 1;
  const items: string[] = [];
  for (;;) {
    const item = tokens[i];
    if (item === undefined) return { ...empty, value: items.join(","), used: i - at, open: true };
    if (!isValueToken(item)) {
      return { ...empty, error: { message: "Expected a value in the list.", start: item.start, end: item.end } };
    }
    items.push(item.text);
    i += 1;
    const next = tokens[i];
    if (next?.kind === "symbol" && next.text === ",") {
      i += 1;
      continue;
    }
    if (parenthesised) {
      if (next === undefined) return { ...empty, value: items.join(","), used: i - at, open: true };
      if (next.kind === "symbol" && next.text === ")") {
        i += 1;
        break;
      }
      return { ...empty, error: { message: 'Expected "," or ")" in the list.', start: next.start, end: next.end } };
    }
    break;
  }
  return { ...empty, value: items.join(","), used: i - at };
}

// WHAT:  Parses the typed filter into rules and the mode they combine in.
// HOW:   Stops at the first problem and reports where it is, so the bar can
//        underline it; an empty text is a valid, empty filter.
export function parseFilterExpr(source: string, columns: readonly FilterColumn[]): ParseResult {
  const { tokens, error } = tokenize(source);
  if (error !== null) return { ok: false, errors: [error] };
  if (tokens.length === 0) return { ok: true, rules: [], mode: "all" };
  const fail = (message: string, start: number, end: number): ParseResult => ({ ok: false, errors: [{ message, start, end }] });
  const rules: FilterRule[] = [];
  let mode: FilterMode | null = null;
  let at = 0;
  for (;;) {
    const columnToken = tokens[at];
    if (columnToken === undefined) return fail("Expected a column name after the connector.", source.length, source.length);
    if (columnToken.kind === "symbol" || columnToken.kind === "string") {
      return fail("Expected a column name.", columnToken.start, columnToken.end);
    }
    const column = resolveColumn(columnToken.text, columns);
    if (column === null) {
      const near = columns.find((c) => c.name.toLowerCase().startsWith(columnToken.text.toLowerCase()));
      return fail(`There is no column "${columnToken.text}"${near ? `. Did you mean "${near.name}"?` : "."}`, columnToken.start, columnToken.end);
    }
    at += 1;
    if (tokens[at] === undefined) return fail(`Expected an operator after "${column}".`, source.length, source.length);
    const operator = readOperator(tokens, at, tokens[at]?.end ?? source.length);
    if (operator.error !== null) return { ok: false, errors: [operator.error] };
    if (operator.op === null) {
      return fail(`Finish the operator (${operator.expects.join(" or ")}).`, source.length, source.length);
    }
    at += operator.used;
    let op = operator.op;
    let value = "";
    if (NEEDS_VALUE[op]) {
      const read = readValue(tokens, at, op);
      if (read.error !== null) return { ok: false, errors: [read.error] };
      if (read.value === null || read.open) {
        const where = tokens[at]?.start ?? source.length;
        return fail(read.open ? 'Close the list with ")".' : `Expected a value after ${OP_TEXT[op]}.`, where, source.length);
      }
      at += read.used;
      value = read.value;
      if (read.bareNull && (op === "eq" || op === "ne")) {
        op = op === "eq" ? "is_null" : "is_not_null";
        value = "";
      }
    }
    rules.push({ column, op, value });
    const connector = tokens[at];
    if (connector === undefined) break;
    const kind: FilterMode | null = wordIs(connector, "and") ? "all" : wordIs(connector, "or") ? "any" : null;
    if (kind === null) return fail('Expected "and" or "or".', connector.start, connector.end);
    if (mode !== null && mode !== kind) return fail('Use "and" or "or" throughout, not both. Use the filter popover for mixed logic.', connector.start, connector.end);
    mode = kind;
    at += 1;
  }
  return { ok: true, rules, mode: mode ?? "all" };
}

const BARE_NAME = /^[A-Za-z_][\w$]*$/;
const BARE_VALUE = /^(?:-?\d+(?:\.\d+)?|[A-Za-z_][\w@.:-]*)$/;

function formatName(name: string): string {
  return BARE_NAME.test(name) && !KEYWORDS.has(name.toLowerCase()) ? name : `"${name.replace(/"/g, '""')}"`;
}

function formatValue(value: string): string {
  return BARE_VALUE.test(value) && !KEYWORDS.has(value.toLowerCase()) ? value : `'${value.replace(/'/g, "''")}'`;
}

// WHAT:  Rules as the text the bar would accept (round-trips through parseFilterExpr).
export function formatFilterExpr(rules: readonly FilterRule[], mode: FilterMode): string {
  const parts = rules.map((rule) => {
    const name = formatName(rule.column);
    if (!NEEDS_VALUE[rule.op]) return `${name} ${OP_TEXT[rule.op]}`;
    if (rule.op === "in") {
      const items = rule.value.split(",").map((v) => v.trim()).filter((v) => v.length > 0);
      return `${name} in (${items.map(formatValue).join(", ")})`;
    }
    return `${name} ${OP_TEXT[rule.op]} ${formatValue(rule.value.trim())}`;
  });
  return parts.join(mode === "any" ? " or " : " and ");
}

export type SuggestionKind = "column" | "operator" | "keyword";

export interface Suggestion {
  label: string;
  /// Text that replaces the word under the cursor.
  insert: string;
  detail?: string;
  kind: SuggestionKind;
}

export interface Suggestions {
  /// The span of the word being completed.
  from: number;
  to: number;
  options: Suggestion[];
}

type Slot = "column" | "operator" | "value" | "connector";

interface Position {
  slot: Slot;
  /// The column the current rule is about, once typed.
  column: string | null;
  /// Words that would finish a half-typed operator.
  expects: string[];
}

// WHAT:  Where in the grammar the completed tokens leave the cursor.
function positionAfter(tokens: readonly Token[], columns: readonly FilterColumn[]): Position {
  let at = 0;
  for (;;) {
    const columnToken = tokens[at];
    if (columnToken === undefined) return { slot: "column", column: null, expects: [] };
    const column = resolveColumn(columnToken.text, columns);
    at += 1;
    if (tokens[at] === undefined) return { slot: "operator", column, expects: [] };
    const operator = readOperator(tokens, at, columnToken.end);
    if (operator.op === null) return { slot: "operator", column, expects: operator.expects };
    at += operator.used;
    if (NEEDS_VALUE[operator.op]) {
      if (tokens[at] === undefined) return { slot: "value", column, expects: [] };
      const read = readValue(tokens, at, operator.op);
      if (read.error !== null || read.open) return { slot: "value", column, expects: [] };
      at += read.used;
    }
    if (tokens[at] === undefined) return { slot: "connector", column, expects: [] };
    at += 1;
  }
}

const OPERATOR_ORDER: readonly FilterOp[] = ["eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "starts_with", "ends_with", "in", "is_null", "is_not_null"];

// WHAT:  Completions for the word under the cursor: columns, then operators the
//        column's type allows, then and / or.
// HOW:   `opsFor` narrows the operator list to the column's type; omitted = all.
export function suggestFilter(
  source: string,
  cursor: number,
  columns: readonly FilterColumn[],
  opsFor?: (typeName: string | undefined) => readonly FilterOp[],
): Suggestions {
  const head = source.slice(0, cursor);
  const { tokens, error } = tokenize(head);
  const none: Suggestions = { from: cursor, to: cursor, options: [] };
  // Inside an unfinished quote the user is typing a value; nothing to complete.
  if (error !== null) return none;
  const last = tokens[tokens.length - 1];
  const typing = last?.kind === "word" && last.end === cursor;
  const done = typing ? tokens.slice(0, -1) : tokens;
  const partial = typing ? last.text.toLowerCase() : "";
  const from = typing ? last.start : cursor;
  const position = positionAfter(done, columns);
  const matches = (label: string) => partial.length === 0 || label.toLowerCase().startsWith(partial);
  const options: Suggestion[] = [];
  if (position.slot === "column") {
    for (const column of columns) {
      if (matches(column.name)) options.push({ label: column.name, insert: `${formatName(column.name)} `, ...(column.typeName ? { detail: column.typeName } : {}), kind: "column" });
    }
    for (const column of columns) {
      if (!matches(column.name) && partial.length > 0 && column.name.toLowerCase().includes(partial)) {
        options.push({ label: column.name, insert: `${formatName(column.name)} `, ...(column.typeName ? { detail: column.typeName } : {}), kind: "column" });
      }
    }
  } else if (position.slot === "operator") {
    if (position.expects.length > 0) {
      for (const word of position.expects) {
        if (matches(word)) options.push({ label: word, insert: word === "null" || word === "not null" ? word : `${word} `, kind: "keyword" });
      }
    } else {
      const typeName = columns.find((c) => c.name === position.column)?.typeName;
      const allowed = opsFor ? opsFor(typeName) : OPERATOR_ORDER;
      for (const op of OPERATOR_ORDER) {
        if (!allowed.includes(op)) continue;
        const text = OP_TEXT[op];
        if (matches(text)) options.push({ label: text, insert: NEEDS_VALUE[op] ? `${text} ` : text, kind: "operator" });
      }
    }
  } else if (position.slot === "connector") {
    for (const word of ["and", "or"]) {
      if (matches(word)) options.push({ label: word, insert: `${word} `, kind: "keyword" });
    }
  }
  return { from, to: cursor, options };
}
