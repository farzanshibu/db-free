// SOT: query-format, formatter-dialect-map, mongo-json-pretty, json-reindent, format-around-shell-commands
import { format as formatSql, type SqlLanguage } from "sql-formatter";
import type { Engine } from "@/lib/bindings";
import { parseJson } from "@/lib/json";
import { splitShellBlocks } from "@/lib/shellCommands";
import { dialectOf } from "@/lib/sqlCatalog";

// WHAT:  Format for every language the editor speaks: SQL in the engine's own
//        dialect, and JSON commands (Mongo, Elasticsearch…) pretty-printed.
// WHY:   One formatter for everything reflowed MySQL backticks and Postgres
//        `::` casts wrongly, and left JSON command documents on one line.
// HOW:   SQL goes through sql-formatter with the dialect the engine's adapter
//        family maps to (`FAMILY_DIALECTS`). JSON is re-indented token by token
//        rather than parsed and printed again: a 64-bit id must come back digit
//        for digit and a command's keys in the order they were written (the
//        server reads the command name from the first key).
// WHERE: src/features/editor/QueryPane.tsx (Format), src/lib/sqlCatalog.ts

export type FormatResult = { ok: true; text: string } | { ok: false; reason: string };

export interface FormatOptions {
  /// Keep lists and expressions on one line (Settings → Condense SQL When Formatting).
  condense: boolean;
}

/// `{{name}}` placeholders pass through the formatter as one token.
const TEMPLATE_PARAM = String.raw`\{\{\s*[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)?\s*\}\}`;

export function formatQuery(text: string, engine: Engine, options: FormatOptions): FormatResult {
  if (text.trim().length === 0) return { ok: false, reason: "There is nothing to format." };
  const dialect = dialectOf(engine);
  if (dialect.flavor !== null) return formatSqlAroundCommands(text, engine, dialect.formatter ?? "sql", options);
  return formatJsonText(text);
}

// WHAT:  SQL with the engine's shell commands (`\d users`, `.tables`) left as typed.
// WHY:   The formatter knows SQL only; handed a command line it would split it or refuse
//        the whole script. The text between the commands is formatted block by block.
function formatAroundCommands(blocks: ReturnType<typeof splitShellBlocks>, language: SqlLanguage, options: FormatOptions): FormatResult {
  const out: string[] = [];
  for (const block of blocks) {
    if (block.shell || block.text.trim().length === 0) {
      out.push(block.text);
      continue;
    }
    const formatted = formatSqlText(block.text, language, options);
    if (!formatted.ok) return formatted;
    out.push(formatted.text);
  }
  return { ok: true, text: out.join("\n") };
}

function formatSqlAroundCommands(text: string, engine: Engine, language: SqlLanguage, options: FormatOptions): FormatResult {
  const blocks = splitShellBlocks(text, engine);
  return blocks.some((block) => block.shell) ? formatAroundCommands(blocks, language, options) : formatSqlText(text, language, options);
}

function formatSqlText(text: string, language: SqlLanguage, options: FormatOptions): FormatResult {
  try {
    return {
      ok: true,
      text: formatSql(text, {
        language,
        keywordCase: "upper",
        expressionWidth: options.condense ? 120 : 50,
        paramTypes: { named: [":"], custom: [{ regex: TEMPLATE_PARAM }] },
      }),
    };
  } catch (raw) {
    return { ok: false, reason: raw instanceof Error ? `Could not format: ${raw.message}` : "Could not format this statement." };
  }
}

// WHAT:  JSON command documents, one or several separated by blank lines.
function formatJsonText(text: string): FormatResult {
  const trimmed = text.trim();
  if (/^(db[.[]|show\s|use\s)/.test(trimmed)) return { ok: false, reason: "Shell statements are run as written; Format reflows SQL and JSON commands." };
  const chunks = trimmed.split(/\n\s*\n/);
  const out: string[] = [];
  for (const [index, chunk] of chunks.entries()) {
    const body = chunk.trim();
    if (body.length === 0) continue;
    const pretty = prettyJson(body);
    if (pretty === null) {
      return { ok: false, reason: chunks.length > 1 ? `Statement ${index + 1} is not valid JSON.` : "Format reflows SQL and JSON commands; this is neither." };
    }
    out.push(pretty);
  }
  return { ok: true, text: out.join("\n\n") };
}

const INLINE_ARRAY_LIMIT = 60;

// WHAT:  Re-indents valid JSON without parsing its values.
// HOW:   Strings are copied verbatim; whitespace outside them is dropped and
//        rebuilt. Arrays of plain values that fit on a line stay on it.
// RETURNS: null when `text` is not valid JSON.
export function prettyJson(text: string): string | null {
  const parsed = parseJson(text);
  if (parsed === undefined || parsed === null || typeof parsed !== "object") return null;
  const out: string[] = [];
  let depth = 0;
  let i = 0;
  const newline = () => out.push(`\n${"  ".repeat(depth)}`);
  while (i < text.length) {
    const c = text.charAt(i);
    if (/\s/.test(c)) {
      i += 1;
    } else if (c === '"') {
      const end = stringEnd(text, i);
      out.push(text.slice(i, end));
      i = end;
    } else if (c === "{" || c === "[") {
      const close = nextSignificant(text, i + 1);
      const inline = c === "[" ? inlineArray(text, i) : null;
      if (text.charAt(close) === (c === "{" ? "}" : "]")) {
        out.push(c === "{" ? "{}" : "[]");
        i = close + 1;
      } else if (inline !== null) {
        out.push(inline.text);
        i = inline.end;
      } else {
        out.push(c);
        depth += 1;
        newline();
        i += 1;
      }
    } else if (c === "}" || c === "]") {
      depth = Math.max(0, depth - 1);
      newline();
      out.push(c);
      i += 1;
    } else if (c === ",") {
      out.push(",");
      newline();
      i += 1;
    } else if (c === ":") {
      out.push(": ");
      i += 1;
    } else {
      out.push(c);
      i += 1;
    }
  }
  return out.join("");
}

/// Index just past the closing quote of the string that starts at `start`.
function stringEnd(text: string, start: number): number {
  let i = start + 1;
  while (i < text.length) {
    const c = text.charAt(i);
    if (c === "\\") i += 2;
    else if (c === '"') return i + 1;
    else i += 1;
  }
  return text.length;
}

function nextSignificant(text: string, from: number): number {
  let i = from;
  while (i < text.length && /\s/.test(text.charAt(i))) i += 1;
  return i;
}

// WHAT:  `[1, "a", true]` when the array holds only plain values and fits on a line.
function inlineArray(text: string, open: number): { text: string; end: number } | null {
  const items: string[] = [];
  let i = open + 1;
  let start = nextSignificant(text, i);
  i = start;
  while (i < text.length) {
    const c = text.charAt(i);
    if (c === '"') {
      i = stringEnd(text, i);
    } else if (c === "{" || c === "[") {
      return null;
    } else if (c === "," || c === "]") {
      const item = text.slice(start, i).trim();
      if (item.length > 0) items.push(item);
      if (c === "]") {
        const joined = `[${items.join(", ")}]`;
        return joined.length <= INLINE_ARRAY_LIMIT ? { text: joined, end: i + 1 } : null;
      }
      i += 1;
      start = i;
    } else {
      i += 1;
    }
  }
  return null;
}
