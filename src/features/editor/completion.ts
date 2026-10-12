// SOT: editor-completion-engine, clause-aware-completion, template-var-completion, mongo-shell-completion, redis-completion-bridge, shell-command-completion-bridge
import type { Engine, ShellCommand, Snippet } from "@/lib/bindings";
import { redisCompletionAt } from "@/lib/redisCommands";
import { shellCompletion } from "@/lib/shellCommands";
import type { SqlCatalog } from "@/lib/sqlCatalog";
import { snippetPreview } from "@/lib/snippets";
import { clauseAt, inStringOrComment, mentionedTables, statementAround, templateAt, tokenize, wantOf, wordAtCaret, type MentionedTable, type Want } from "./sqlScan";

// WHAT:  Decides what the editor's suggestion list shows for a caret position.
// WHY:   A flat list of every table, column and keyword answers "what could be
//        here" badly. After FROM the user wants tables; after WHERE the columns
//        of the tables this statement names; inside `{{` the query's variables;
//        after `db.users.` the shell's collection methods.
// HOW:   Pure: text, caret and the schema in, ranked items out. The statement
//        around the caret is read with `sqlScan` (clause, mentioned tables,
//        aliases); the language of the engine picks the vocabulary.
// WHERE: src/features/editor/SqlEditor.tsx (renders the list and applies a choice)

export type CompletionKind = "snippet" | "table" | "column" | "keyword" | "function" | "variable" | "command" | "method";

export interface CompletionItem {
  label: string;
  detail: string;
  kind: CompletionKind;
  /// Set for `snippet` items: accepting expands the body instead of inserting the label.
  snippet?: Snippet;
  /// Inserted instead of the label (a function call with its parentheses).
  insert?: string;
  /// Where the caret lands inside `insert`; the end when absent.
  caret?: number;
  /// A longer description, shown as the row's tooltip (Redis commands).
  info?: string;
}

export interface SchemaTable {
  name: string;
  columns: string[];
}

/// A `{{name}}` the surrounding page defines (a dashboard variable).
export interface TemplateVariable {
  name: string;
  /// A date range: `{{name.start}}` and `{{name.end}}` are valid too.
  range?: boolean;
}

// WHAT:  The `{{names}}` a page offers, from the flat token list a dashboard builds
//        (`name`, and `name.start` / `name.end` for a date range).
// WHY:   The dashboard owns its variables and the editor owns the dropdown; this is
//        the one place that knows both spellings, so neither imports the other.
// HOW:   A token with a dot is the `.start` / `.end` of a range; once a name has
//        either, it is offered with its two parts.
export function templateVariablesFromTokens(tokens: readonly { token: string }[]): TemplateVariable[] {
  const ranges = new Set<string>();
  for (const { token } of tokens) {
    const dot = token.lastIndexOf(".");
    if (dot > 0) ranges.add(token.slice(0, dot));
  }
  const names = new Set<string>();
  for (const { token } of tokens) {
    if (!token.includes(".")) names.add(token);
  }
  return [...names].map((name) => (ranges.has(name) ? { name, range: true } : { name }));
}

export type EditorLanguage = "sql" | "mongo" | "redis" | "other";

export interface CompletionInput {
  text: string;
  caret: number;
  /// The engine the editor is connected to: which shell commands (`\dt`, `.tables`, `help`) it offers.
  engine: Engine;
  tables: readonly SchemaTable[];
  catalog: SqlCatalog | null;
  language: EditorLanguage;
  snippets: readonly Snippet[];
  defaultSchema: string | undefined;
  variables: readonly TemplateVariable[];
}

export interface CompletionResult {
  from: number;
  to: number;
  items: CompletionItem[];
}

export const MAX_COMPLETIONS = 100;

const MONGO_COLLECTION_METHODS = [
  "find", "findOne", "aggregate", "countDocuments", "estimatedDocumentCount", "distinct", "insertOne", "insertMany", "updateOne",
  "updateMany", "replaceOne", "deleteOne", "deleteMany", "findOneAndUpdate", "findOneAndDelete", "findOneAndReplace", "createIndex",
  "dropIndex", "getIndexes", "drop", "stats", "validate", "exists",
] as const;
const MONGO_DATABASE_METHODS = [
  "getCollectionNames", "getCollectionInfos", "getCollection", "getSiblingDB", "runCommand", "adminCommand", "stats", "version", "serverStatus",
  "hostInfo", "currentOp", "getUsers", "getRoles", "getProfilingStatus", "createCollection", "dropDatabase",
] as const;
/// What `show` can be followed by in the console.
const MONGO_SHOW_WORDS = ["dbs", "databases", "collections", "tables", "users", "roles", "profile", "logs"] as const;
const DATABASE_METHOD_NAMES: ReadonlySet<string> = new Set(MONGO_DATABASE_METHODS);
const MONGO_CURSOR_METHODS = ["sort", "limit", "skip", "project", "hint", "collation", "maxTimeMS", "count", "toArray", "pretty"] as const;
/// The JSON command keys whose value is a collection name.
const MONGO_COLLECTION_KEYS = "find|aggregate|count|insert|update|delete|distinct|drop|createIndexes|dropIndexes|listIndexes|collStats|findAndModify|validate";

// ---- ranking -----------------------------------------------------------------

/// 0 prefix, 1 a later word of the name starts with it, 2 contains it; null: no match.
function matchRank(label: string, partial: string): number | null {
  if (partial.length === 0) return 0;
  const l = label.toLowerCase();
  const p = partial.toLowerCase();
  if (l.startsWith(p)) return 0;
  if (l.split(/[_.\- ]/).some((word) => word.startsWith(p))) return 1;
  return l.includes(p) ? 2 : null;
}

interface Candidate {
  item: CompletionItem;
  /// Lower sorts first among equally good text matches.
  group: number;
}

function rank(partial: string, candidates: readonly Candidate[]): CompletionItem[] {
  const scored: { item: CompletionItem; match: number; group: number; order: number }[] = [];
  candidates.forEach((candidate, order) => {
    const match = matchRank(candidate.item.label, partial);
    if (match !== null) scored.push({ item: candidate.item, match, group: candidate.group, order });
  });
  scored.sort((a, b) => a.match - b.match || a.group - b.group || a.order - b.order);
  return scored.slice(0, MAX_COMPLETIONS).map((s) => s.item);
}

/// Keeps the user's casing for names they have started typing in lower case.
function casedLike(name: string, partial: string): string {
  return partial.length > 0 && partial === partial.toLowerCase() ? name.toLowerCase() : name.toUpperCase();
}

// ---- schema lookups ------------------------------------------------------------

class Schema {
  private readonly byFull = new Map<string, SchemaTable>();
  private readonly byBare = new Map<string, SchemaTable[]>();

  constructor(
    readonly tables: readonly SchemaTable[],
    private readonly defaultSchema: string | undefined,
  ) {
    for (const table of tables) {
      this.byFull.set(table.name.toLowerCase(), table);
      const bare = (table.name.split(".").pop() ?? table.name).toLowerCase();
      this.byBare.set(bare, [...(this.byBare.get(bare) ?? []), table]);
    }
  }

  // WHAT:  A table from a name as written: `users`, `public.users`, `"Users"`.
  resolve(name: string): SchemaTable | undefined {
    const lower = name.toLowerCase();
    const exact = this.byFull.get(lower);
    if (exact !== undefined) return exact;
    if (lower.includes(".")) {
      return this.tables.find((t) => t.name.toLowerCase().endsWith(`.${lower}`) || lower.endsWith(`.${t.name.toLowerCase()}`));
    }
    const candidates = this.byBare.get(lower) ?? [];
    const preferred = this.defaultSchema === undefined ? undefined : candidates.find((t) => t.name.toLowerCase().startsWith(`${this.defaultSchema?.toLowerCase() ?? ""}.`));
    return preferred ?? candidates[0];
  }

  /// The name to offer for a table: bare when it lives in the default schema.
  display(table: SchemaTable): string {
    const prefix = this.defaultSchema === undefined ? "" : `${this.defaultSchema}.`;
    return prefix.length > 0 && table.name.toLowerCase().startsWith(prefix.toLowerCase()) ? table.name.slice(prefix.length) : table.name;
  }

  schemaNames(): string[] {
    const names = new Set<string>();
    for (const table of this.tables) {
      const dot = table.name.lastIndexOf(".");
      if (dot > 0) names.add(table.name.slice(0, dot));
    }
    return [...names];
  }
}

// ---- entry point -----------------------------------------------------------------

export function computeCompletions(input: CompletionInput): CompletionResult | null {
  const { text, caret } = input;
  const variables = variableCompletions(input);
  if (variables !== null) return variables;
  const shell = shellCompletion({ text, caret, engine: input.engine, statements: input.language === "sql" });
  // A `\dt` line is only ever a command: the SQL vocabulary has nothing to say on it.
  if (shell?.claimed === true) return shell.items.length === 0 ? null : { from: shell.from, to: caret, items: shell.items.map(shellItem) };
  return withShellCommands(languageResult(input), shell === null ? null : { from: shell.from, items: shell.items }, caret);
}

function languageResult(input: CompletionInput): CompletionResult | null {
  switch (input.language) {
    case "redis":
      return redisResult(input);
    case "mongo":
      return mongoResult(input);
    case "sql":
    case "other":
      return sqlResult(input, input.text, input.caret);
  }
}

// ---- shell commands ----------------------------------------------------------------------

function briefSummary(text: string): string {
  return text.length > 26 ? `${text.slice(0, 25)}…` : text;
}

function shellItem(command: ShellCommand): CompletionItem {
  return { label: command.name, detail: briefSummary(command.summary), kind: "command", info: `${command.syntax}\n${command.summary}` };
}

// WHAT:  The engine's shell commands (`DESCRIBE KEYSPACES`, `help`) beside the language's own suggestions.
// HOW:   When both start at the same place they share one list, a command replacing a keyword
//        of the same name (it carries the description); when the command is already more than
//        one word long the language's list has nothing to add and the commands stand alone.
function withShellCommands(base: CompletionResult | null, shell: { from: number; items: readonly ShellCommand[] } | null, caret: number): CompletionResult | null {
  if (shell?.items.length === undefined || shell.items.length === 0) return base;
  const commands = shell.items.map(shellItem);
  if (base?.from !== shell.from) return { from: shell.from, to: caret, items: commands };
  const named = new Set(commands.map((c) => c.label.toLowerCase()));
  const items = [...commands, ...base.items.filter((item) => !named.has(item.label.toLowerCase()))].slice(0, MAX_COMPLETIONS);
  return { ...base, items };
}

// ---- {{variables}} -------------------------------------------------------------------

function variableCompletions(input: CompletionInput): CompletionResult | null {
  const context = templateAt(input.text, input.caret);
  if (context === null) return null;
  const seen = new Map<string, TemplateVariable>(input.variables.map((v) => [v.name, v]));
  // Names the script already uses are offered too: `{{name}}` typed once is typed once.
  for (const match of input.text.matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_]*)(?:\.(?:start|end))?\s*\}\}/g)) {
    const name = match[1];
    if (name !== undefined && !seen.has(name)) seen.set(name, { name });
  }
  const candidates: Candidate[] = [];
  if (context.base === null) {
    for (const variable of seen.values()) {
      candidates.push({ item: { label: variable.name, detail: variable.range === true ? "date range" : "variable", kind: "variable" }, group: 0 });
    }
  } else {
    const variable = seen.get(context.base);
    if (variable?.range === true) {
      for (const part of ["start", "end"]) candidates.push({ item: { label: part, detail: `${variable.name} ${part}`, kind: "variable" }, group: 0 });
    }
  }
  const items = rank(context.partial, candidates);
  // Inside `{{` the list is the variables or nothing: no keywords, no tables.
  return items.length === 0 ? null : { from: context.from, to: input.caret, items };
}

// ---- Redis --------------------------------------------------------------------------------

function redisResult(input: CompletionInput): CompletionResult | null {
  const lineStart = input.text.lastIndexOf("\n", input.caret - 1) + 1;
  const found = redisCompletionAt(input.text.slice(lineStart, input.caret));
  if (found === null) return null;
  // The row is narrow: a short syntax hint beside the name, the whole syntax and the description in the tooltip.
  const brief = (text: string) => (text.length > 24 ? `${text.slice(0, 23)}…` : text);
  return {
    from: lineStart + found.from,
    to: input.caret,
    items: found.items.map((c) => ({ label: c.label, detail: brief(c.detail), kind: "command" as const, info: `${`${c.label} ${c.detail}`.trim()}\n${c.info}` })),
  };
}

// ---- MongoDB ---------------------------------------------------------------------------------

function mongoResult(input: CompletionInput): CompletionResult | null {
  const { text, caret } = input;
  const lineStart = text.lastIndexOf("\n", caret - 1) + 1;
  const line = text.slice(lineStart, caret);
  const collections = (): Candidate[] => input.tables.map((t) => ({ item: { label: bareName(t.name), detail: "collection", kind: "table" as const }, group: 1 }));
  const method = (name: string, detail: string): Candidate => ({ item: { label: name, detail, kind: "method" as const, insert: `${name}()`, caret: name.length + 1 }, group: 0 });

  // db.users.find().sort(…): the cursor's methods after a closing paren.
  const chain = /\)\s*\.([A-Za-z]*)$/.exec(line);
  if (chain !== null) return mongoItems(caret, chain[1] ?? "", MONGO_CURSOR_METHODS.map((m) => method(m, "cursor")));
  // db.users.<method>
  const onCollection = /\bdb\.([A-Za-z0-9_$]+)\.([A-Za-z]*)$/.exec(line);
  if (onCollection !== null && !DATABASE_METHOD_NAMES.has(onCollection[1] ?? "")) {
    return mongoItems(caret, onCollection[2] ?? "", MONGO_COLLECTION_METHODS.map((m) => method(m, "collection")));
  }
  // db.<collection or database method>
  const onDb = /\bdb\.([A-Za-z0-9_$]*)$/.exec(line);
  if (onDb !== null) {
    return mongoItems(caret, onDb[1] ?? "", [...collections(), ...MONGO_DATABASE_METHODS.map((m) => method(m, "database"))]);
  }
  // show collections / show dbs, use <db>
  const show = /^\s*show\s+([A-Za-z]*)$/.exec(line);
  if (show !== null) {
    return mongoItems(caret, show[1] ?? "", MONGO_SHOW_WORDS.map((word) => ({ item: { label: word, detail: "show", kind: "keyword" as const }, group: 0 })));
  }
  // {"find": "us… — the value of a command key is a collection name.
  const jsonValue = new RegExp(`"(?:${MONGO_COLLECTION_KEYS})"\\s*:\\s*"([A-Za-z0-9_.\\-]*)$`).exec(line);
  if (jsonValue !== null) return mongoItems(caret, jsonValue[1] ?? "", collections());
  const word = wordAtCaret(text, caret);
  if (word.partial.length === 0 || word.path.length > 0) return null;
  const snippets = input.snippets.map((sn) => ({ item: { label: sn.prefix, detail: sn.name.length > 0 ? sn.name : snippetPreview(sn.body), kind: "snippet" as const, snippet: sn }, group: 0 }));
  return mongoItems(caret, word.partial, [...snippets, ...collections(), ...["db", "show", "use"].map((k) => ({ item: { label: k, detail: "shell", kind: "keyword" as const }, group: 2 }))]);
}

function mongoItems(caret: number, partial: string, candidates: readonly Candidate[]): CompletionResult | null {
  const items = rank(partial, candidates);
  return items.length === 0 ? null : { from: caret - partial.length, to: caret, items };
}

function bareName(name: string): string {
  return name.includes(".") && !name.startsWith(".") ? (name.split(".").slice(1).join(".") || name) : name;
}

// ---- SQL ---------------------------------------------------------------------------------------

function sqlResult(input: CompletionInput, text: string, caret: number): CompletionResult | null {
  const bounds = statementAround(text, caret);
  const statement = text.slice(bounds.from, bounds.to);
  const tokens = tokenize(statement, bounds.from);
  if (input.language === "sql" && inStringOrComment(tokens, caret)) return null;
  const schema = new Schema(input.tables, input.defaultSchema);
  const word = wordAtCaret(text, caret);
  const mentioned = mentionedTables(tokens);

  if (word.path.length > 0) return qualifiedResult(schema, mentioned, word.path, word.partial, word.from, caret);
  if (word.partial.length === 0) return null;

  const want = wantOf(clauseAt(tokens, caret));
  return plainResult(input, schema, mentioned, want, word.partial, word.from, caret);
}

// WHAT:  `alias.`, `table.`, `schema.`, `schema.table.`: the columns of one table,
//        or the tables of one schema.
function qualifiedResult(schema: Schema, mentioned: readonly MentionedTable[], path: readonly string[], partial: string, from: number, caret: number): CompletionResult | null {
  const candidates: Candidate[] = [];
  const first = path.join(".");
  const alias = mentioned.find((m) => m.alias !== null && m.alias.toLowerCase() === first.toLowerCase());
  const table = schema.resolve(alias?.name ?? first);
  if (table !== undefined) {
    for (const column of table.columns) candidates.push({ item: { label: column, detail: schema.display(table), kind: "column" }, group: 0 });
  }
  // A schema name lists its tables (the part after the schema is what gets inserted).
  const prefix = `${first.toLowerCase()}.`;
  for (const candidate of schema.tables) {
    if (!candidate.name.toLowerCase().startsWith(prefix)) continue;
    const rest = candidate.name.slice(prefix.length);
    if (!rest.includes(".")) candidates.push({ item: { label: rest, detail: `${candidate.columns.length} cols`, kind: "table" }, group: 1 });
  }
  const items = rank(partial, candidates);
  return items.length === 0 ? null : { from, to: caret, items };
}

function plainResult(input: CompletionInput, schema: Schema, mentioned: readonly MentionedTable[], want: Want, partial: string, from: number, caret: number): CompletionResult | null {
  const candidates: Candidate[] = [];
  // Group numbers per clause: the thing the clause asks for sorts ahead of the rest.
  const order = want === "tables" ? { snippet: 0, table: 0, mentioned: 5, column: 6, keyword: 1, fn: 3 } : want === "columns" ? { snippet: 4, table: 5, mentioned: 0, column: 3, keyword: 2, fn: 1 } : want === "none" ? { snippet: 4, table: 6, mentioned: 5, column: 6, keyword: 1, fn: 0 } : { snippet: 0, table: 2, mentioned: 3, column: 4, keyword: 1, fn: 5 };

  for (const snippet of input.snippets) {
    candidates.push({ item: { label: snippet.prefix, detail: snippet.name.length > 0 ? snippet.name : snippetPreview(snippet.body), kind: "snippet", snippet }, group: order.snippet });
  }
  const mentionedTablesFound = mentioned.map((m) => schema.resolve(m.name)).filter((t): t is SchemaTable => t !== undefined);
  const seenColumns = new Map<string, { item: CompletionItem; tables: string[] }>();
  const addColumn = (column: string, table: SchemaTable, group: number) => {
    const known = seenColumns.get(`${group}:${column}`);
    if (known !== undefined) {
      known.tables.push(schema.display(table));
      known.item.detail = `${known.tables.length} tables`;
      return;
    }
    const item: CompletionItem = { label: column, detail: schema.display(table), kind: "column" };
    seenColumns.set(`${group}:${column}`, { item, tables: [schema.display(table)] });
    candidates.push({ item, group });
  };

  if (want !== "tables") {
    for (const table of mentionedTablesFound) for (const column of table.columns) addColumn(column, table, order.mentioned);
  }
  // Tables, ahead of the default schema's own when the clause is asking for them.
  const inDefault = (t: SchemaTable) => (input.defaultSchema !== undefined && t.name.toLowerCase().startsWith(`${input.defaultSchema.toLowerCase()}.`) ? 0 : 1);
  [...input.tables].sort((a, b) => inDefault(a) - inDefault(b)).forEach((table) => {
    candidates.push({ item: { label: schema.display(table), detail: `${table.columns.length} cols`, kind: "table" }, group: order.table + inDefault(table) * 0.5 });
  });
  // Columns of tables the statement does not name: only when nothing narrows them down.
  if (want !== "tables" && mentionedTablesFound.length === 0) {
    for (const table of input.tables) for (const column of table.columns) addColumn(column, table, order.column);
  }
  const catalog = input.catalog;
  if (catalog !== null && input.language === "sql") {
    if (want !== "tables") {
      for (const fn of catalog.functions) {
        const name = casedLike(fn, partial);
        candidates.push({ item: { label: name, detail: "function", kind: "function", insert: `${name}()`, caret: name.length + 1 }, group: order.fn });
      }
    }
    for (const keyword of catalog.keywords) candidates.push({ item: { label: keyword, detail: "keyword", kind: "keyword" }, group: order.keyword });
  }
  const items = rank(partial, candidates);
  return items.length === 0 ? null : { from, to: caret, items };
}
