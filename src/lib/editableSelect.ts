// SOT: editable-select-detection, sql-tokenizer, result-edit-target, select-list-mapping

// WHAT:  Decides whether a query result is one table's rows, and if so which
//        table and which result columns are that table's columns.
// WHY:   Editing a result writes back by primary key to a real table. That is
//        only safe when every result row is exactly one table row: a join, a
//        GROUP BY, DISTINCT, UNION or a subquery in FROM breaks that, and a
//        computed column (`upper(name) AS name`) must never be written back
//        even though its name matches a real column.
// HOW:   A tokenizer that skips string literals, quoted identifiers, comments
//        and dollar-quoted bodies, then a walk over the top level (parenthesis
//        depth 0) of one SELECT. Anything it does not recognise is read-only,
//        with a reason the results pane shows; a false "no" costs a click, a
//        false "yes" writes to the wrong row.
// WHERE: src/features/editor/ResultsPane.tsx

export interface EditTarget {
  /// The table as written: 1 to 3 dotted parts, unquoted. The caller resolves
  /// it against the catalog (case, default schema).
  parts: string[];
  /// Whether each part was quoted (quoted names keep their case).
  quoted: boolean[];
  /// `*` or `t.*` in the select list: every result column is a table column.
  star: boolean;
  /// Result column name -> table column name, for plain column references
  /// (`id`, `t.name`, `name AS n`).
  columns: ReadonlyMap<string, string>;
  /// Aliases of computed items (`upper(name) AS name`): never written back,
  /// even when `*` also brings in a real column of that name.
  computed: ReadonlySet<string>;
}

export type SelectAnalysis = { editable: true; target: EditTarget } | { editable: false; reason: string };

type TokenKind = "word" | "quoted" | "string" | "number" | "punct";

interface Token {
  kind: TokenKind;
  /// Keywords and bare identifiers as written; quoted identifiers unquoted.
  text: string;
}

// WHAT:  SQL text -> tokens, dropping whitespace and comments.
// HOW:   '…' strings ('' escapes), "…" / `…` / […] identifiers, -- and /* */
//        comments, $tag$…$tag$ bodies. Operators come out one character at a
//        time, which is all the walk below needs.
export function tokenize(sql: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const ch = sql.charAt(i);
    const next = sql.charAt(i + 1);
    if (/\s/.test(ch)) {
      i += 1;
    } else if (ch === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      i = end < 0 ? n : end + 1;
    } else if (ch === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? n : end + 2;
    } else if (ch === "'") {
      let j = i + 1;
      let text = "";
      while (j < n) {
        const c = sql.charAt(j);
        if (c === "\\" && j + 1 < n) {
          text += sql.charAt(j + 1);
          j += 2;
        } else if (c === "'" && sql.charAt(j + 1) === "'") {
          text += "'";
          j += 2;
        } else if (c === "'") {
          break;
        } else {
          text += c;
          j += 1;
        }
      }
      tokens.push({ kind: "string", text });
      i = j + 1;
    } else if (ch === '"' || ch === "`" || ch === "[") {
      const close = ch === "[" ? "]" : ch;
      let j = i + 1;
      let text = "";
      while (j < n) {
        const c = sql.charAt(j);
        if (c === close && sql.charAt(j + 1) === close) {
          text += close;
          j += 2;
        } else if (c === close) {
          break;
        } else {
          text += c;
          j += 1;
        }
      }
      tokens.push({ kind: "quoted", text });
      i = j + 1;
    } else if (ch === "$" && /[A-Za-z_$]/.test(next)) {
      const tag = /^\$[A-Za-z0-9_]*\$/.exec(sql.slice(i));
      if (tag) {
        const end = sql.indexOf(tag[0], i + tag[0].length);
        tokens.push({ kind: "string", text: end < 0 ? sql.slice(i + tag[0].length) : sql.slice(i + tag[0].length, end) });
        i = end < 0 ? n : end + tag[0].length;
      } else {
        tokens.push({ kind: "punct", text: ch });
        i += 1;
      }
    } else if (/[A-Za-z_]/.test(ch)) {
      const word = /^[A-Za-z_][A-Za-z0-9_$#@]*/.exec(sql.slice(i))?.[0] ?? ch;
      tokens.push({ kind: "word", text: word });
      i += word.length;
    } else if (/[0-9]/.test(ch) || (ch === "." && /[0-9]/.test(next))) {
      const num = /^[0-9]*\.?[0-9]+(?:[eE][+-]?[0-9]+)?/.exec(sql.slice(i))?.[0] ?? ch;
      tokens.push({ kind: "number", text: num });
      i += num.length;
    } else {
      tokens.push({ kind: "punct", text: ch });
      i += 1;
    }
  }
  return tokens;
}

function isWord(t: Token | undefined, ...words: string[]): boolean {
  return t?.kind === "word" && words.includes(t.text.toUpperCase());
}

function isPunct(t: Token | undefined, text: string): boolean {
  return t?.kind === "punct" && t.text === text;
}

function isIdent(t: Token | undefined): boolean {
  return t !== undefined && (t.kind === "quoted" || (t.kind === "word" && !RESERVED.has(t.text.toUpperCase())));
}

/// Words that can never be a table or column name here; they end a clause.
const RESERVED = new Set(["SELECT", "FROM", "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FETCH", "FOR", "UNION", "INTERSECT", "EXCEPT", "MINUS", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "NATURAL", "ON", "USING", "AS", "WINDOW", "QUALIFY", "INTO", "DISTINCT", "ALL", "TOP", "WITH", "LATERAL", "APPLY", "TABLESAMPLE", "SAMPLE"]);
/// Clauses that may follow a single-table FROM without changing what a row is.
const TAIL_CLAUSES = new Set(["WHERE", "ORDER", "LIMIT", "OFFSET", "FETCH", "FOR", "WINDOW", "QUALIFY"]);

/// Splits a token run on top-level commas.
function splitTopLevel(tokens: readonly Token[]): Token[][] {
  const out: Token[][] = [[]];
  let depth = 0;
  for (const t of tokens) {
    if (isPunct(t, "(")) depth += 1;
    if (isPunct(t, ")")) depth -= 1;
    if (depth === 0 && isPunct(t, ",")) out.push([]);
    else out[out.length - 1]?.push(t);
  }
  return out;
}

/// `a`, `a.b`, `a.b.c` -> the parts, or null when the run is anything else.
function dotted(tokens: readonly Token[]): Token[] | null {
  const parts: Token[] = [];
  for (let i = 0; i < tokens.length; i += 1) {
    const t = tokens[i];
    if (i % 2 === 0) {
      if (t === undefined || !isIdent(t)) return null;
      parts.push(t);
    } else if (!isPunct(t, ".")) {
      return null;
    }
  }
  return tokens.length % 2 === 1 ? parts : null;
}

function readOnly(reason: string): SelectAnalysis {
  return { editable: false, reason };
}

// WHAT:  One statement -> an edit target, or the reason there is none.
export function analyzeSelect(sql: string): SelectAnalysis {
  const all = tokenize(sql);
  while (isPunct(all.at(-1), ";")) all.pop();
  if (all.some((t) => isPunct(t, ";"))) return readOnly("the script has more than one statement");
  if (isWord(all[0], "WITH")) return readOnly("queries with WITH (common table expressions) are not editable");
  if (!isWord(all[0], "SELECT")) return readOnly("only SELECT results are editable");

  // Top-level positions of the clause keywords.
  let depth = 0;
  let from = -1;
  let fromEnd = all.length;
  for (let i = 1; i < all.length; i += 1) {
    const t = all[i];
    if (isPunct(t, "(")) depth += 1;
    else if (isPunct(t, ")")) depth -= 1;
    if (depth !== 0 || t?.kind !== "word") continue;
    const word = t.text.toUpperCase();
    if (word === "UNION" || word === "INTERSECT" || word === "EXCEPT" || word === "MINUS") return readOnly(`${word} results are not editable`);
    if (word === "GROUP" || word === "HAVING") return readOnly("grouped results are not editable");
    if (word === "INTO") return readOnly("SELECT … INTO is not editable");
    if (word === "FROM" && from < 0) from = i;
    else if (from >= 0 && fromEnd === all.length && TAIL_CLAUSES.has(word)) fromEnd = i;
  }
  if (depth !== 0) return readOnly("the statement could not be read (unbalanced parentheses)");
  if (from < 0) return readOnly("the query reads no table");

  // Select list: DISTINCT changes what a row is; ALL / TOP n do not.
  let listStart = 1;
  if (isWord(all[listStart], "DISTINCT", "DISTINCTROW", "UNIQUE")) return readOnly("DISTINCT results are not editable");
  if (isWord(all[listStart], "ALL")) listStart += 1;
  if (isWord(all[listStart], "TOP")) {
    listStart += 1;
    if (isPunct(all[listStart], "(")) {
      while (listStart < from && !isPunct(all[listStart], ")")) listStart += 1;
      listStart += 1;
    } else {
      listStart += 1;
    }
    if (isWord(all[listStart], "PERCENT")) listStart += 1;
    if (isWord(all[listStart], "WITH") && isWord(all[listStart + 1], "TIES")) listStart += 2;
  }

  // FROM: exactly one table, optionally aliased.
  const fromTokens = all.slice(from + 1, fromEnd);
  if (fromTokens.some((t) => isPunct(t, "("))) return readOnly("subqueries, table functions and table hints in FROM are not editable");
  if (fromTokens.some((t) => isPunct(t, ",") || isWord(t, "JOIN", "CROSS", "NATURAL", "APPLY", "LATERAL"))) return readOnly("joins are not editable");
  const tableTokens = isWord(fromTokens[0], "ONLY") ? fromTokens.slice(1) : fromTokens;
  let nameEnd = 1;
  while (isPunct(tableTokens[nameEnd], ".") && nameEnd + 1 < tableTokens.length) nameEnd += 2;
  const nameParts = dotted(tableTokens.slice(0, nameEnd));
  if (!nameParts || nameParts.length > 3) return readOnly("the FROM clause could not be read");
  const afterName = tableTokens.slice(nameEnd);
  const aliasTokens = isWord(afterName[0], "AS") ? afterName.slice(1) : afterName;
  if (aliasTokens.length > 1 || (aliasTokens.length === 1 && !isIdent(aliasTokens[0]))) return readOnly("the FROM clause could not be read");

  // Select items: `*`, `t.*`, or a plain column reference with an optional alias.
  let star = false;
  const columns = new Map<string, string>();
  const computed = new Set<string>();
  for (const item of splitTopLevel(all.slice(listStart, from))) {
    if (item.length === 1 && isPunct(item[0], "*")) {
      star = true;
      continue;
    }
    if (item.length >= 3 && isPunct(item.at(-1), "*") && isPunct(item.at(-2), ".") && dotted(item.slice(0, -2)) !== null) {
      star = true;
      continue;
    }
    // Longest dotted run first, then `[AS] alias`.
    let end = 1;
    while (isPunct(item[end], ".") && end + 1 < item.length) end += 2;
    const ref = dotted(item.slice(0, end));
    const column = ref?.at(-1);
    if (!ref || !column) {
      const last = item.at(-1);
      if (item.length > 1 && last !== undefined && isIdent(last)) computed.add(last.text);
      continue;
    }
    const rest = item.slice(end);
    const alias = isWord(rest[0], "AS") ? rest.slice(1) : rest;
    const name = alias[0];
    const last = item.at(-1);
    if (alias.length === 0) columns.set(column.text, column.text);
    else if (alias.length === 1 && name !== undefined && (isIdent(name) || name.kind === "string")) columns.set(name.text, column.text);
    else if (last !== undefined && isIdent(last)) computed.add(last.text);
  }

  return {
    editable: true,
    target: { parts: nameParts.map((p) => p.text), quoted: nameParts.map((p) => p.kind === "quoted"), star, columns, computed },
  };
}
