// SOT: chat-suggestions, starter-prompts, rotating-placeholder, suggestion-templates
import type { ChatMention } from "@/lib/bindings";
import { mentionOf, type MentionCandidate } from "./mentions";
import type { IconName } from "@/lib/icons";

// WHAT:  Starter prompts built from the database's own tables.
// WHY:   "Ask about your data" is a blank page; "Find duplicates in orders" is
//        one click. Pairing a few real tables with a few templates gives
//        suggestions that are specific to this database and different on each
//        visit, without calling a model.
// HOW:   Pure and seeded: the same seed gives the same picks, so the empty state
//        does not reshuffle on every render, and "More ideas" just bumps the seed.
// WHERE: src/features/chat/ChatTab.tsx

export interface StarterSuggestion {
  label: string;
  prompt: string;
  icon: IconName;
  /// The table the prompt is about, attached when the suggestion is sent.
  mention: ChatMention;
}

interface Template {
  label: (table: string) => string;
  prompt: (token: string) => string;
  icon: IconName;
}

const TEMPLATES: readonly Template[] = [
  { label: (t) => `Build a dashboard for ${t}`, prompt: (t) => `Build a dashboard for the @${t} table`, icon: "chart-bar" },
  { label: (t) => `Write a query for ${t}`, prompt: (t) => `Write a query that answers a useful question about @${t}`, icon: "code" },
  { label: (t) => `Describe ${t}`, prompt: (t) => `Describe @${t}: what each column is for and what the table is used for`, icon: "info" },
  { label: (t) => `How does ${t} relate to other tables?`, prompt: (t) => `Which tables does @${t} relate to, and through which keys? Draw it.`, icon: "chart-relationship" },
  { label: (t) => `Sample rows from ${t}`, prompt: (t) => `Show me 10 sample rows from @${t}`, icon: "rows" },
  { label: (t) => `Which indexes would help ${t}?`, prompt: (t) => `Which indexes should I add to @${t} for better query performance?`, icon: "flash" },
  { label: (t) => `Summarise ${t}`, prompt: (t) => `Give me a statistical summary of @${t}: counts, averages, minimums and maximums for the numeric columns`, icon: "gauge" },
  { label: (t) => `Find duplicates in ${t}`, prompt: (t) => `Write a query that finds duplicate rows in @${t}`, icon: "search" },
  { label: (t) => `Check ${t} for bad data`, prompt: (t) => `Check @${t} for data quality problems: nulls where there should be values, orphaned foreign keys, odd values`, icon: "shield" },
  { label: (t) => `Recent activity in ${t}`, prompt: (t) => `Show the most recent rows in @${t}, ordered by its most likely timestamp column`, icon: "clock" },
  { label: (t) => `Break ${t} down by category`, prompt: (t) => `Group and count the rows of @${t} by its most useful categorical column, and chart it`, icon: "chart" },
  { label: (t) => `Find missing values in ${t}`, prompt: (t) => `Count the NULLs in each column of @${t} so I can see where data is missing`, icon: "alert" },
  { label: (t) => `Growth of ${t} over time`, prompt: (t) => `Chart how the row count of @${t} has grown over time, by day or month`, icon: "activity" },
  { label: (t) => `Join ${t} with its neighbours`, prompt: (t) => `Write a query that joins @${t} with its related tables into one useful view`, icon: "link" },
  { label: (t) => `Design a workflow around ${t}`, prompt: (t) => `Create a workflow that checks @${t} for problems and reports what it finds`, icon: "flow" },
  { label: (t) => `Document ${t}`, prompt: (t) => `Write documentation for @${t}: purpose, columns, relationships and common query patterns`, icon: "book" },
];

/// A small deterministic generator, so a seed always shuffles the same way.
function seeded(seed: number): () => number {
  let state = (seed >>> 0) + 0x6d2b79f5;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function shuffled<T>(items: readonly T[], random: () => number): T[] {
  const copy = [...items];
  for (let i = copy.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    const a = copy[i];
    const b = copy[j];
    if (a !== undefined && b !== undefined) {
      copy[i] = b;
      copy[j] = a;
    }
  }
  return copy;
}

/// Three (by default) different tables, each with a different template.
export function pickSuggestions(tables: readonly MentionCandidate[], seed: number, count = 3): StarterSuggestion[] {
  const random = seeded(seed);
  const picked = shuffled(tables, random).slice(0, count);
  const templates = shuffled(TEMPLATES, random);
  const out: StarterSuggestion[] = [];
  picked.forEach((table, index) => {
    const template = templates[index % templates.length];
    if (template === undefined) return;
    out.push({
      label: template.label(table.label),
      prompt: template.prompt(table.label),
      icon: template.icon,
      mention: mentionOf(table),
    });
  });
  return out;
}

/// What an empty database or a missing catalogue falls back to.
export const GENERIC_STARTERS: readonly { label: string; prompt: string; icon: IconName }[] = [
  { label: "What tables are in this database?", prompt: "What tables are in this database and how do they relate?", icon: "database" },
  { label: "Show the biggest tables", prompt: "Show me the 10 biggest tables by row count", icon: "table" },
  { label: "Find tables without a primary key", prompt: "Find any table missing a primary key", icon: "key" },
  { label: "Audit the schema", prompt: "Audit this schema and tell me what I should fix first", icon: "shield" },
];

/// The line under the title, cycling while the box is empty.
export const ROTATING_HINTS: readonly string[] = [
  "Ask about your schema, your data, a row or a cell",
  "Ask me to build a dashboard for your database",
  "Ask me to write a SQL query",
  "Ask me to explain how your tables relate",
  "Ask me to help debug a slow query",
  "Type @ to point at a table, a saved query or a dashboard",
];
