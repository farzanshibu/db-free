// SOT: snippets, snippet-registry, builtin-snippets, snippet-languages
import type { Engine, Snippet } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { engineMeta } from "@/lib/engines";

// WHAT:  Tab-expandable snippets for the query editor: the built-in set per
//        command language plus the user's own (AppSettings.snippets).
// WHY:   `sel⇥` → a SELECT skeleton with the cursor on the column list is the
//        fastest way to write the statements everyone types a hundred times.
// HOW:   A body is CodeMirror snippet syntax: `${name}` is a field Tab walks
//        through (same name = linked), `${}` the final caret. A user snippet
//        with the same prefix and language as a built-in replaces it.
// WHERE: src/features/editor/SqlEditor.tsx (expansion + dropdown),
//        src/features/settings/SnippetsSection.tsx (user snippets)
export type SnippetLanguage = "sql" | "cypher" | "mongo" | "redis" | "any";

export const SNIPPET_LANGUAGES = [
  { value: "sql", label: "SQL" },
  { value: "cypher", label: "Cypher" },
  { value: "mongo", label: "MongoDB command" },
  { value: "redis", label: "Redis command" },
  { value: "any", label: "Every language" },
] satisfies readonly { value: SnippetLanguage; label: string }[];

export function isSnippetLanguage(value: string): value is SnippetLanguage {
  return SNIPPET_LANGUAGES.some((l) => l.value === value);
}

// WHAT:  The snippet language an engine's editor speaks, or null when none of
//        the built-in sets apply (only "any" user snippets are offered then).
export function snippetLanguageOf(engine: Engine): SnippetLanguage | null {
  const language = engineMeta(engine).commandLanguage;
  if (language === "SQL") return "sql";
  if (language === "Cypher") return "cypher";
  const family = ENGINE_FACTS[engine].family;
  if (family === "mongodb") return "mongo";
  if (family === "redis") return "redis";
  return null;
}

const s = (language: SnippetLanguage, prefix: string, name: string, body: string): Snippet => ({ language, prefix, name, body });

export const BUILTIN_SNIPPETS: readonly Snippet[] = [
  s("sql", "sel", "SELECT … FROM … WHERE", "SELECT ${columns} FROM ${table} WHERE ${condition};"),
  s("sql", "ins", "INSERT INTO … VALUES", "INSERT INTO ${table} (${columns}) VALUES (${values});"),
  s("sql", "upd", "UPDATE … SET … WHERE", "UPDATE ${table} SET ${column} = ${value} WHERE ${condition};"),
  s("sql", "del", "DELETE FROM … WHERE", "DELETE FROM ${table} WHERE ${condition};"),
  s("sql", "cte", "WITH … AS (…) SELECT", "WITH ${name} AS (\n  SELECT ${columns} FROM ${table}\n)\nSELECT * FROM ${name};"),
  s("sql", "join", "SELECT … JOIN … ON", "SELECT ${columns}\nFROM ${left} a\nJOIN ${right} b ON b.${key} = a.${key};"),
  s("sql", "cnt", "SELECT COUNT(*)", "SELECT COUNT(*) FROM ${table};"),
  s("sql", "grp", "GROUP BY with count", "SELECT ${column}, COUNT(*) AS n\nFROM ${table}\nGROUP BY ${column}\nORDER BY n DESC;"),
  s("cypher", "match", "MATCH … RETURN", "MATCH (${n}:${Label})\nWHERE ${condition}\nRETURN ${n}\nLIMIT 25;"),
  s("cypher", "rel", "MATCH a relationship", "MATCH (a:${From})-[r:${TYPE}]->(b:${To})\nRETURN a, r, b\nLIMIT 25;"),
  s("mongo", "find", "find command", '{ "find": "${collection}", "filter": { ${} }, "limit": 50 }'),
  s("mongo", "agg", "aggregate pipeline", '{ "aggregate": "${collection}", "pipeline": [\n  { "$match": { ${} } },\n  { "$group": { "_id": "$${field}", "n": { "$sum": 1 } } }\n], "cursor": {} }'),
  s("mongo", "cnt", "count command", '{ "count": "${collection}", "query": { ${} } }'),
  s("redis", "get", "GET a key", "GET ${key}"),
  s("redis", "set", "SET with expiry", "SET ${key} ${value} EX ${seconds}"),
  s("redis", "hgetall", "HGETALL a hash", "HGETALL ${key}"),
  s("redis", "scan", "SCAN by pattern", "SCAN 0 MATCH ${pattern}* COUNT 100"),
];

// WHAT:  Every snippet the editor should offer for one language: the user's
//        first, then the built-ins they did not override.
export function snippetsFor(language: SnippetLanguage | null, user: readonly Snippet[]): Snippet[] {
  const applies = (snippet: Snippet) => snippet.language === "any" || snippet.language === language;
  const out: Snippet[] = [];
  const taken = new Set<string>();
  // One snippet per prefix, first wins: user before built-in, earlier before later.
  for (const snippet of [...user, ...BUILTIN_SNIPPETS]) {
    if (snippet.prefix.length === 0 || !applies(snippet) || taken.has(snippet.prefix)) continue;
    taken.add(snippet.prefix);
    out.push(snippet);
  }
  return out;
}

/// The snippet whose prefix is exactly the word before the caret, if any.
export function snippetForWord(word: string, list: readonly Snippet[]): Snippet | undefined {
  return word.length === 0 ? undefined : list.find((snippet) => snippet.prefix === word);
}

/// One-line preview of a body for a list or dropdown: fields shown by name.
export function snippetPreview(body: string): string {
  return body.replace(/[$#]\{(?:\d+:)?([^}]*)\}/g, "$1").replace(/\s+/g, " ").trim();
}
