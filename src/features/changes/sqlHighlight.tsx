// SOT: sql-highlight, highlighted-sql-view, sql-tokenizer
import { cn } from "@/lib/cn";

// WHAT:  SQL text with keywords, strings, numbers and comments coloured by the
//        theme's syntax tokens.
// WHY:   The pending statements are read before they run; colour makes the
//        `WHERE` that decides which row changes stand out from its values.
// HOW:   A small scanner, not a parser — enough for the INSERT / UPDATE / DELETE /
//        ALTER the panel shows. Anything it does not recognise stays plain.
// WHERE: src/features/changes/PendingChangesPanel.tsx
type Kind = "keyword" | "string" | "number" | "comment" | "ident" | "plain";

interface Piece {
  kind: Kind;
  text: string;
}

const KEYWORDS: ReadonlySet<string> = new Set([
  "select", "from", "where", "and", "or", "not", "null", "is", "in", "insert", "into", "values", "update", "set", "delete", "alter", "table",
  "add", "drop", "column", "create", "index", "unique", "primary", "key", "foreign", "references", "constraint", "default", "returning",
  "begin", "commit", "rollback", "transaction", "on", "as", "like", "ilike", "limit", "offset", "order", "by", "asc", "desc", "true", "false",
  "rename", "to", "type", "check", "cascade", "restrict", "if", "exists", "savepoint", "release", "truncate", "view", "schema", "using",
]);

const WORD = /[A-Za-z_][A-Za-z0-9_$]*/y;
const NUMBER = /-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/y;

export function tokenizeSql(sql: string): Piece[] {
  const pieces: Piece[] = [];
  let plain = "";
  const flush = () => {
    if (plain.length > 0) pieces.push({ kind: "plain", text: plain });
    plain = "";
  };
  const push = (kind: Kind, text: string) => {
    flush();
    pieces.push({ kind, text });
  };
  let i = 0;
  while (i < sql.length) {
    const ch = sql[i] ?? "";
    const next = sql[i + 1];
    if (ch === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      const stop = end === -1 ? sql.length : end;
      push("comment", sql.slice(i, stop));
      i = stop;
    } else if (ch === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      const stop = end === -1 ? sql.length : end + 2;
      push("comment", sql.slice(i, stop));
      i = stop;
    } else if (ch === "'") {
      let j = i + 1;
      while (j < sql.length) {
        if (sql[j] === "'" && sql[j + 1] === "'") j += 2;
        else if (sql[j] === "\\" && j + 1 < sql.length) j += 2;
        else if (sql[j] === "'") {
          j += 1;
          break;
        } else j += 1;
      }
      push("string", sql.slice(i, j));
      i = j;
    } else if (ch === '"' || ch === "`" || ch === "[") {
      const close = ch === "[" ? "]" : ch;
      const end = sql.indexOf(close, i + 1);
      const stop = end === -1 ? sql.length : end + 1;
      push("ident", sql.slice(i, stop));
      i = stop;
    } else if (/[0-9]/.test(ch) || (ch === "-" && /[0-9]/.test(next ?? "") && /[\s=(,]/.test(sql[i - 1] ?? " "))) {
      NUMBER.lastIndex = i;
      const m = NUMBER.exec(sql);
      const text = m?.[0] ?? ch;
      push("number", text);
      i += text.length;
    } else if (/[A-Za-z_]/.test(ch)) {
      WORD.lastIndex = i;
      const m = WORD.exec(sql);
      const text = m?.[0] ?? ch;
      if (KEYWORDS.has(text.toLowerCase())) push("keyword", text);
      else plain += text;
      i += text.length;
    } else {
      plain += ch;
      i += 1;
    }
  }
  flush();
  return pieces;
}

const CLASS: Record<Kind, string> = {
  keyword: "text-syntax-keyword font-medium",
  string: "text-syntax-string",
  number: "text-syntax-number",
  comment: "text-syntax-comment italic",
  ident: "text-syntax-type",
  plain: "",
};

export function HighlightedSql({ sql, className }: { sql: string; className?: string }) {
  return (
    <pre className={cn("selectable font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all text-foreground", className)}>
      {tokenizeSql(sql).map((piece, index) => (
        <span key={index} className={CLASS[piece.kind]}>
          {piece.text}
        </span>
      ))}
    </pre>
  );
}
