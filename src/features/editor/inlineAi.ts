// SOT: inline-ai-prompt-shape, ai-code-extraction, inline-insert-placement, ai-prompt-comment, ask-ai-about-sql

// WHAT:  The text work behind the editor's ⌘I prompt: what to ask the model,
//        how to pull the code out of its reply, and how it lands in the buffer.
// WHY:   The model answers in markdown prose around a code block. The editor
//        wants only the code, placed so it never glues itself to the line above
//        or below, optionally headed by the instruction it came from.
// HOW:   Pure functions over strings, so the placement rules are plain to read
//        and the component only deals with state and events.
// WHERE: src/features/editor/InlineAiPrompt.tsx, src/features/editor/QueryPane.tsx

export interface InlineRequest {
  /// What the user asked for.
  instruction: string;
  /// The highlighted text to rewrite; empty when writing at the caret.
  selection: string;
  /// The language the answer must be in: "SQL", "Cypher", "Command"…
  language: string;
  /// What the engine is called ("PostgreSQL").
  engine: string;
}

// WHAT:  The instruction sent to the model for an inline request.
// WHY:   One code block and nothing else: anything more has to be cut out of the
//        text before it can go into the editor.
export function inlinePrompt(request: InlineRequest): string {
  const { instruction, selection, language, engine } = request;
  const shape = `Reply with only the ${language} in one fenced code block, no explanation.`;
  if (selection.trim().length > 0) {
    return `Rewrite this ${engine} ${language} according to the instruction. ${shape}\n\nInstruction: ${instruction}\n\n${language} to rewrite:\n\`\`\`\n${selection}\n\`\`\``;
  }
  return `Write ${engine} ${language} for the instruction. ${shape}\n\nInstruction: ${instruction}`;
}

// WHAT:  The code in a model reply: the last complete fenced block, else a block
//        that is still being written, else the whole reply.
// WHY:   While the reply streams the closing fence has not arrived; the preview
//        should show the code so far, not the fence and the prose.
export function extractCode(markdown: string): string {
  const closed = [...markdown.matchAll(/```[^\n`]*\n([\s\S]*?)```/g)];
  const last = closed[closed.length - 1];
  if (last?.[1] !== undefined) return last[1].trim();
  const open = /```[^\n`]*\n([\s\S]*)$/.exec(markdown);
  if (open?.[1] !== undefined) return open[1].trim();
  return markdown.includes("```") ? "" : markdown.trim();
}

/// The instruction on one line, short enough to read as a comment.
export function commentLine(prefix: string, instruction: string): string {
  const flat = instruction.replace(/\s+/g, " ").trim();
  const shown = flat.length > 160 ? `${flat.slice(0, 157)}…` : flat;
  return `${prefix} ${shown}`;
}

export interface Placement {
  from: number;
  to: number;
  insert: string;
}

// WHAT:  Where the answer goes and exactly what is written there.
// HOW:   A selection is replaced. At a bare caret the code goes on its own
//        line(s): a newline is added before it when the caret follows text, and
//        after it when text follows the caret. With `comment` set, the
//        instruction heads the code as a comment line.
export function placeAnswer(doc: string, from: number, to: number, code: string, comment: string | null): Placement {
  const body = comment === null ? code : `${comment}\n${code}`;
  if (to > from) return { from, to, insert: body };
  const before = from > 0 ? doc.charAt(from - 1) : "";
  const after = from < doc.length ? doc.charAt(from) : "";
  const lead = before !== "" && before !== "\n" ? "\n" : "";
  const trail = after !== "" && after !== "\n" ? "\n" : "";
  return { from, to, insert: `${lead}${body}${trail}` };
}

// WHAT:  The prompt "Ask AI about this SQL" hands to the chat.
// HOW:   The highlighted text when there is some, else the statement the caret is
//        in; the last error, when there is one, rides along.
export function askAboutPrompt(sql: string, language: string, error: string | null): string {
  const lines = [`Explain this ${language} and point out anything wrong or slow:`, "", "```", sql.trim(), "```"];
  if (error !== null) lines.push("", `It failed with: ${error}`);
  return lines.join("\n");
}

/// The prompt "Fix with AI" hands to the chat for a failed statement.
export function fixErrorPrompt(sql: string, language: string, error: string, position: { line: number; column: number } | null): string {
  const where = position === null ? "" : ` (line ${position.line}, column ${position.column})`;
  return [`This ${language} failed${where}. Find the cause and give me the corrected statement.`, "", "```", sql.trim(), "```", "", `Error: ${error}`].join("\n");
}
