// SOT: chat-prompts, ask-ai-entry-points, ai-seed-prompts
import { aiCanReadRows } from "./access";
import { tokenOf } from "./mentions";
import type { AppSettings } from "@/lib/bindings";

// WHAT:  Ready-made prompts for the places that hand work to the chat.
// WHY:   Dashboards, workflows and diagrams each have an empty state that wants
//        a "let AI build it" button; the grid wants "Ask AI about this row"; the
//        tables list wants "Generate dashboard". They all end in
//        `useWorkspace.getState().openChat(connectionId, prompt)`, and the
//        prompt is the only thing that differs, so it is written once here.
// HOW:   A table written as `@schema.table` becomes a real mention when the chat
//        receives the prompt (the chat matches `@word` tokens against the
//        catalogue), so the model is handed the table's definition.
// WHERE: callers: features/dashboards, features/workflows, features/diagrams,
//        features/grid, features/schema, features/editor

/// "Generate dashboard" on a table's menu.
export function dashboardForTablePrompt(table: string): string {
  return `Build a dashboard for the @${tokenOf(table)} table: headline metrics first, then the charts that tell its story, then a table of recent rows.`;
}

/// The empty dashboard.
export function emptyDashboardPrompt(): string {
  return "Build me a dashboard for this database. Look at the schema, pick the tables that matter most, and give me headline metrics, a few charts and a table.";
}

/// The empty workflow.
export function emptyWorkflowPrompt(): string {
  return "Create a workflow for this database: a short sequence of checks or maintenance steps that is useful here. Look at the schema first and tell me what each step does.";
}

/// The empty diagram.
export function emptyDiagramPrompt(): string {
  return "Create a diagram of this database's schema: the main tables, their columns and the relations between them.";
}

/// "Ask AI" on a table or a selection of tables.
export function askAboutTablePrompt(table: string): string {
  return `Tell me about the @${tokenOf(table)} table: what it holds, how it relates to other tables, and anything in its design worth knowing.`;
}

/// "Ask AI about this SQL" from the editor.
export function askAboutSqlPrompt(sql: string): string {
  return `Explain what this statement does, and point out anything wrong or slow about it:\n\n\`\`\`sql\n${sql.trim()}\n\`\`\``;
}

// WHAT:  "Ask AI" on a grid row.
// WHY:   The row's values are sent to the provider as part of the prompt, which a
//        schema-only chat promises never to do — so the caller must check
//        `canAskAboutRow(settings)` and fall back to the table-level prompt when
//        it is false.
export function askAboutRowPrompt(table: string, columns: readonly string[], values: readonly string[]): string {
  const lines = columns.map((column, index) => `- ${column}: ${values[index] ?? ""}`);
  return `This is a row of @${tokenOf(table)}:\n${lines.join("\n")}\n\nWhat can you tell me about it? Point out anything unusual and how it relates to other rows or tables.`;
}

/// Whether row values may go into a prompt.
export function canAskAboutRow(settings: AppSettings | null): boolean {
  return aiCanReadRows(settings);
}
