// SOT: sql-token-scan, statement-at-caret, clause-at-caret, mentioned-tables, template-var-context

// WHAT:  A small SQL reader for the editor's suggestions: where the caret's
//        statement starts and ends, which clause the caret is in, which tables the
//        statement names, and whether the caret sits in a string, a comment or a
//        `{{variable}}`.
// WHY:   Suggestions depend on context. After FROM the editor should offer
//        tables, after WHERE the columns of the tables in this statement, and
//        inside a string literal nothing at all.
// HOW:   One pass over the text yields tokens with their paren depth; strings
//        and comments become tokens of their own, so a keyword inside either is
//        never read as one. Pure functions, no editor state.
// WHERE: src/features/editor/completion.ts

export type TokenKind = "word" | "quoted" | "string" | "number" | "comment" | "open" | "close" | "punct";

export interface Token {
  kind: TokenKind;
  /// The text as written; a quoted identifier keeps its quotes.
  text: string;
  start: number;
  end: number;
  /// Parenthesis depth the token sits at (an open paren is at the outer depth).
  depth: number;
}

const WORD_START = /[A-Za-z_$@#\u0080-￿]/;
const WORD_PART = /[A-Za-z0-9_$@#\u0080-￿]/;

export function tokenize(text: string, offset = 0): Token[] {
  const out: Token[] = [];
  let depth = 0;
  let i = 0;
  const push = (kind: TokenKind, start: number, end: number, at: number) => out.push({ kind, text: text.slice(start, end), start: start + offset, end: end + offset, depth: at });
  while (i < text.length) {
    const c = text.charAt(i);
    const next = text.charAt(i + 1);
    if (/\s/.test(c)) {
      i += 1;
    } else if (c === "-" && next === "-") {
      const end = text.indexOf("\n", i);
      const stop = end < 0 ? text.length : end;
      push("comment", i, stop, depth);
      i = stop;
    } else if (c === "/" && next === "*") {
      const end = text.indexOf("*/", i + 2);
      const stop = end < 0 ? text.length : end + 2;
      push("comment", i, stop, depth);
      i = stop;
    } else if (c === "'") {
      const stop = skipQuoted(text, i, "'");
      push("string", i, stop, depth);
      i = stop;
    } else if (c === '"' || c === "`") {
      const stop = skipQuoted(text, i, c);
      push("quoted", i, stop, depth);
      i = stop;
    } else if (c === "[") {
      // A T-SQL bracket identifier; elsewhere a lone `[` is just punctuation and the scan moves on.
      const close = text.indexOf("]", i);
      const stop = close < 0 ? i + 1 : close + 1;
      push(close < 0 ? "punct" : "quoted", i, stop, depth);
      i = stop;
    } else if (c === "$" && /^\$[A-Za-z_]*\$/.test(text.slice(i))) {
      const tag = /^\$[A-Za-z_]*\$/.exec(text.slice(i))?.[0] ?? "$$";
      const close = text.indexOf(tag, i + tag.length);
      const stop = close < 0 ? text.length : close + tag.length;
      push("string", i, stop, depth);
      i = stop;
    } else if (c === "(") {
      push("open", i, i + 1, depth);
      depth += 1;
      i += 1;
    } else if (c === ")") {
      depth = Math.max(0, depth - 1);
      push("close", i, i + 1, depth);
      i += 1;
    } else if (/[0-9]/.test(c)) {
      let stop = i + 1;
      while (stop < text.length && /[0-9.eE_]/.test(text.charAt(stop))) stop += 1;
      push("number", i, stop, depth);
      i = stop;
    } else if (WORD_START.test(c)) {
      let stop = i + 1;
      while (stop < text.length && WORD_PART.test(text.charAt(stop))) stop += 1;
      push("word", i, stop, depth);
      i = stop;
    } else {
      push("punct", i, i + 1, depth);
      i += 1;
    }
  }
  return out;
}

/// Index just past the closing quote; a doubled quote is an escaped one.
function skipQuoted(text: string, start: number, quote: string): number {
  let i = start + 1;
  while (i < text.length) {
    const ch = text.charAt(i);
    if (ch === "\\" && quote === "'") {
      i += 2;
      continue;
    }
    if (ch === quote) {
      if (text.charAt(i + 1) === quote) {
        i += 2;
        continue;
      }
      return i + 1;
    }
    i += 1;
  }
  return text.length;
}

// WHAT:  The statement the caret is in: from just after the previous top-level
//        `;` to the next one.
export function statementAround(text: string, caret: number): { from: number; to: number } {
  let from = 0;
  let to = text.length;
  for (const token of tokenize(text)) {
    if (token.kind !== "punct" || token.text !== ";" || token.depth !== 0) continue;
    if (token.end <= caret) from = token.end;
    else if (token.start >= caret) {
      to = token.start;
      break;
    }
  }
  return { from, to };
}

/// True when the caret is inside a string literal or a comment, where nothing should be suggested.
export function inStringOrComment(tokens: readonly Token[], caret: number): boolean {
  for (const token of tokens) {
    if (token.kind !== "string" && token.kind !== "comment") continue;
    if (caret <= token.start) continue;
    if (caret < token.end) return true;
    // Right at the end: still inside when the string or comment never closed
    // (a line comment runs to the end of its line).
    if (caret === token.end && !isClosed(token)) return true;
  }
  return false;
}

function isClosed(token: Token): boolean {
  if (token.kind === "comment") return token.text.startsWith("/*") && token.text.length >= 4 && token.text.endsWith("*/");
  const first = token.text.charAt(0);
  return token.text.length > 1 && token.text.endsWith(first);
}

export type Clause =
  | "select"
  | "from"
  | "join"
  | "where"
  | "on"
  | "group"
  | "having"
  | "order"
  | "set"
  | "insert-columns"
  | "values"
  | "into"
  | "table"
  | "other";

/// What the caret is being asked for.
export type Want = "tables" | "columns" | "none" | "any";

const TABLE_VERBS = new Set(["TABLE", "VIEW", "UPDATE", "TRUNCATE", "DESCRIBE", "DESC", "REFERENCES"]);

// WHAT:  The clause the caret is in, at the caret's own parenthesis depth, so a
//        subquery `(SELECT … FROM …)` has a clause of its own.
// HOW:   Walk the tokens before the caret; at each depth remember the latest clause
//        keyword. The parenthesis stack restores the outer clause when a subquery
//        closes, and an open paren right after `INTO <table>` is an INSERT column list.
export function clauseAt(tokens: readonly Token[], caret: number): Clause {
  const stack: Clause[] = ["other"];
  let previous: Token | undefined;
  for (const token of tokens) {
    if (token.start >= caret) break;
    if (token.kind === "comment" || token.kind === "string") continue;
    const top = stack[stack.length - 1] ?? "other";
    if (token.kind === "open") {
      const namesTable = previous !== undefined && (previous.kind === "word" || previous.kind === "quoted");
      stack.push(top === "into" && namesTable ? "insert-columns" : top);
    } else if (token.kind === "close") {
      if (stack.length > 1) stack.pop();
    } else if (token.kind === "word") {
      const next = keywordClause(token.text.toUpperCase(), previous);
      if (next !== null) stack[stack.length - 1] = next;
    }
    previous = token;
  }
  return stack[stack.length - 1] ?? "other";
}

function isWord(token: Token, word: string): boolean {
  return token.kind === "word" && token.text.toUpperCase() === word;
}

function keywordClause(word: string, previous: Token | undefined): Clause | null {
  switch (word) {
    case "SELECT":
    case "DISTINCT":
    case "RETURNING":
      return "select";
    case "INTO":
      return "into";
    case "FROM":
      return "from";
    case "JOIN":
      return "join";
    case "WHERE":
      return "where";
    case "ON":
    case "USING":
      return "on";
    case "HAVING":
      return "having";
    case "SET":
      return "set";
    case "VALUES":
      return "values";
    case "GROUP":
      return "group";
    case "BY":
      return previous !== undefined && isWord(previous, "ORDER") ? "order" : previous !== undefined && isWord(previous, "GROUP") ? "group" : null;
    default:
      return TABLE_VERBS.has(word) ? "table" : null;
  }
}

// WHAT:  What the clause is asking for. Table names after FROM / JOIN / INTO,
//        columns after SELECT / WHERE / ON / GROUP BY / ORDER BY / SET, nothing
//        inside a VALUES list; `any` where either could follow.
export function wantOf(clause: Clause): Want {
  switch (clause) {
    case "from":
    case "join":
    case "into":
    case "table":
      return "tables";
    case "select":
    case "where":
    case "on":
    case "group":
    case "having":
    case "order":
    case "set":
    case "insert-columns":
      return "columns";
    case "values":
      return "none";
    case "other":
      return "any";
  }
}

export interface MentionedTable {
  /// As written, unquoted: `users`, `public.users`.
  name: string;
  alias: string | null;
}

/// Words that follow a table name and are never its alias.
const NOT_ALIAS = new Set([
  "WHERE", "GROUP", "ORDER", "LIMIT", "OFFSET", "HAVING", "UNION", "INTERSECT", "EXCEPT", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER",
  "FULL", "CROSS", "NATURAL", "ON", "USING", "SELECT", "FROM", "SET", "VALUES", "WINDOW", "FETCH", "FOR", "RETURNING", "LATERAL",
  "WITH", "AS", "AND", "OR", "NOT", "INTO", "FORMAT", "SETTINGS", "FINAL", "PREWHERE", "SAMPLE", "QUALIFY", "OUTPUT",
]);

export function unquote(identifier: string): string {
  const first = identifier.charAt(0);
  if (first === '"' || first === "`") return identifier.slice(1, identifier.endsWith(first) && identifier.length > 1 ? -1 : undefined).replaceAll(first + first, first);
  if (first === "[") return identifier.slice(1, identifier.endsWith("]") ? -1 : undefined);
  return identifier;
}

// WHAT:  Every table the statement names after FROM / JOIN / UPDATE / INTO (and
//        after a comma in a FROM list), with its alias.
// HOW:   Reads the whole statement, not just the part before the caret, because
//        `SELECT | FROM users` wants users' columns although FROM comes later.
export function mentionedTables(tokens: readonly Token[]): MentionedTable[] {
  const out: MentionedTable[] = [];
  const live = tokens.filter((t) => t.kind !== "comment");
  let inFromList = false;
  let listDepth = 0;
  // `CREATE INDEX name ON table (…)`: here ON names a table, not a join condition.
  const indexStatement = live[0] !== undefined && isWord(live[0], "CREATE") && live.some((t) => isWord(t, "INDEX"));
  for (let i = 0; i < live.length; i += 1) {
    const token = live[i];
    if (token === undefined) continue;
    const upper = token.kind === "word" ? token.text.toUpperCase() : "";
    const startsList = upper === "FROM" || upper === "JOIN" || upper === "UPDATE" || upper === "INTO" || upper === "TABLE" || (indexStatement && upper === "ON");
    const continuesList: boolean = inFromList && token.kind === "punct" && token.text === "," && token.depth === listDepth;
    if (!startsList && !continuesList) {
      if (token.kind === "word" && inFromList && token.depth <= listDepth && /^(WHERE|GROUP|ORDER|HAVING|LIMIT|UNION|SET|VALUES|ON|USING)$/.test(upper)) inFromList = false;
      continue;
    }
    inFromList = upper === "FROM" || continuesList;
    listDepth = token.depth;
    const parsed = readTableName(live, i + 1);
    if (parsed === null) continue;
    out.push(parsed.table);
    i = parsed.next - 1;
  }
  return out;
}

function readTableName(tokens: readonly Token[], from: number): { table: MentionedTable; next: number } | null {
  const parts: string[] = [];
  let i = from;
  for (;;) {
    const part = tokens[i];
    if (part === undefined || (part.kind !== "word" && part.kind !== "quoted")) break;
    if (parts.length === 0 && part.kind === "word" && NOT_ALIAS.has(part.text.toUpperCase())) break;
    parts.push(unquote(part.text));
    i += 1;
    const dot = tokens[i];
    if (dot?.kind === "punct" && dot.text === ".") {
      i += 1;
      continue;
    }
    break;
  }
  if (parts.length === 0) return null;
  let alias: string | null = null;
  const maybe = tokens[i];
  if (maybe?.kind === "word" && maybe.text.toUpperCase() === "AS") {
    const named = tokens[i + 1];
    if (named !== undefined && (named.kind === "word" || named.kind === "quoted")) {
      alias = unquote(named.text);
      i += 2;
    }
  } else if (maybe !== undefined && (maybe.kind === "quoted" || (maybe.kind === "word" && !NOT_ALIAS.has(maybe.text.toUpperCase())))) {
    alias = unquote(maybe.text);
    i += 1;
  }
  return { table: { name: parts.join("."), alias }, next: i };
}

export interface WordAtCaret {
  /// The identifier being typed, possibly empty.
  partial: string;
  /// Start of `partial` in the text.
  from: number;
  /// The identifier before the dot when the caret follows `qualifier.`; null otherwise.
  qualifier: string | null;
  /// Every part before the last dot (`schema.table.` -> ["schema", "table"]).
  path: string[];
}

// WHAT:  The (possibly dotted) name ending at the caret.
export function wordAtCaret(text: string, caret: number): WordAtCaret {
  let start = caret;
  while (start > 0 && WORD_PART.test(text.charAt(start - 1))) start -= 1;
  const partial = text.slice(start, caret);
  const path: string[] = [];
  let cursor = start;
  while (cursor > 0 && text.charAt(cursor - 1) === ".") {
    let partStart = cursor - 1;
    const closing = text.charAt(partStart - 1);
    if (closing === '"' || closing === "`" || closing === "]") {
      const opener = closing === "]" ? "[" : closing;
      const open = text.lastIndexOf(opener, partStart - 2);
      if (open < 0) break;
      path.unshift(unquote(text.slice(open, partStart)));
      partStart = open;
    } else {
      while (partStart > 0 && WORD_PART.test(text.charAt(partStart - 1))) partStart -= 1;
      if (partStart === cursor - 1) break;
      path.unshift(text.slice(partStart, cursor - 1));
    }
    cursor = partStart;
  }
  return { partial, from: start, qualifier: path.length > 0 ? (path[path.length - 1] ?? null) : null, path };
}

export interface TemplateContext {
  /// What has been typed of the variable name (`from` in `{{from`).
  partial: string;
  from: number;
  /// Set once a dot follows a name: `{{range.` offers `start` / `end`.
  base: string | null;
}

// WHAT:  The `{{variable}}` the caret is inside, if any.
// HOW:   The nearest `{{` before the caret with no `}}` between it and the caret.
export function templateAt(text: string, caret: number): TemplateContext | null {
  const open = text.lastIndexOf("{{", caret);
  if (open < 0) return null;
  const closed = text.indexOf("}}", open);
  if (closed >= 0 && closed < caret) return null;
  const typed = text.slice(open + 2, caret);
  if (!/^\s*[A-Za-z0-9_.]*$/.test(typed)) return null;
  const name = typed.trimStart();
  const from = caret - name.length;
  const dot = name.lastIndexOf(".");
  return dot < 0 ? { partial: name, from, base: null } : { partial: name.slice(dot + 1), from: from + dot + 1, base: name.slice(0, dot) };
}
