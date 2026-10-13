// SOT: shell-command-registry, shell-command-completion, shell-command-line-detection, shell-command-format-split
import type { Engine, ShellCommand, ShellDialectInfo } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { SHELL_DIALECTS } from "@/lib/bindings/ShellCommands.gen";

// WHAT:  What the query tab knows about the structure commands of each engine's own
//        shell (`\dt`, `.tables`, `DESCRIBE KEYSPACES`, `:schema`, `help`): which
//        ones the engine offers, whether the caret is typing one, and which lines
//        of a script are one.
// WHY:   The Rust side translates them (integrations/shell_meta); the editor has to
//        offer them while typing and keep Format from reflowing a `\d users` line.
//        The list itself is generated from the Rust dialects, so a command added
//        there is offered here with no second list to keep.
// HOW:   Pure functions over text. A dialect with a `prefix` (`\`, `.`, `:`) has
//        commands that start with it and end at their line; a dialect without one
//        has word commands (`DESC emp`, `show users`, `help`) matched as a typed
//        prefix of a command's whole name.
// WHERE: src/features/editor/completion.ts (suggestions), src/features/editor/formatQuery.ts,
//        src-tauri/src/integrations/shell_meta (the commands), src/lib/bindings/ShellCommands.gen.ts

export function shellDialectOf(engine: Engine): ShellDialectInfo {
  return SHELL_DIALECTS[ENGINE_FACTS[engine].family];
}

export interface ShellCompletionInput {
  text: string;
  caret: number;
  engine: Engine;
  /// The language cuts scripts at `;` (SQL and its cousins): a command can only begin a
  /// statement, and only the engine's own shell commands are offered, not every statement it runs.
  statements: boolean;
}

export interface ShellCompletion {
  /// Where the typed command begins.
  from: number;
  items: ShellCommand[];
  /// The caret is on a prefix command line (`\d…`): nothing else belongs in the list.
  claimed: boolean;
}

/// A name that stands for something to fill in (`db.<collection>.stats()`): documented, not completed.
function isTemplate(name: string): boolean {
  return name.includes("<");
}

/// Names that are not words to type: a JSON request or a multi-line query shown in help.
function isLiteralText(name: string): boolean {
  return !/^[A-Za-z\\.:!?]/.test(name) || name.includes("\n");
}

// WHAT:  The commands the line under the caret could be the start of.
// HOW:   Prefix dialects: the line (indent aside) starts with the prefix and has no blank
//        yet, so the name is still being typed. Word dialects: what is typed, as a
//        case-insensitive prefix of a command's name, at the start of a statement
//        (or a line, where scripts are not cut at `;`). A command already typed in full is not offered again.
export function shellCompletion(input: ShellCompletionInput): ShellCompletion | null {
  const dialect = shellDialectOf(input.engine);
  const { text, caret } = input;
  const lineStart = text.lastIndexOf("\n", caret - 1) + 1;
  const before = text.slice(lineStart, caret);
  const typed = before.trimStart();
  if (typed.length === 0) return null;
  const from = lineStart + (before.length - typed.length);
  const prefixed = dialect.prefix !== null && typed.startsWith(dialect.prefix);
  if (prefixed && /\s/.test(typed)) return null;
  if (input.statements && !atStatementStart(text, lineStart, dialect.prefix)) return null;

  const wanted = typed.replace(/\s+/g, " ").toLowerCase();
  const candidates = dialect.commands.filter((command) => {
    if (isTemplate(command.name) || isLiteralText(command.name)) return false;
    if (input.statements && command.origin !== "shell") return false;
    const name = command.name.toLowerCase();
    if (name === wanted || !name.startsWith(wanted)) return false;
    // `\dt` is a command; a word dialect's `desc` is not a prefix command.
    return prefixed ? dialect.prefix !== null && command.name.startsWith(dialect.prefix) : true;
  });
  // A prefix line with nothing to offer is still a command line: the caller keeps the SQL vocabulary off it.
  if (candidates.length === 0 && !prefixed) return null;
  // The case the user typed first: `\dt` ahead of `\dT`.
  const exact = candidates.filter((c) => c.name.startsWith(typed));
  const rest = candidates.filter((c) => !c.name.startsWith(typed));
  return { from, items: [...exact, ...rest], claimed: prefixed };
}

// WHAT:  Whether the line starting at `lineStart` begins a statement: nothing before it
//        but blanks, comments and finished statements (or, for a prefix dialect, command lines).
function atStatementStart(text: string, lineStart: number, prefix: string | null): boolean {
  // `end` is the line break that closes the line above; walk up through blank and comment lines.
  let end = lineStart - 1;
  while (end >= 0) {
    const start = end === 0 ? 0 : text.lastIndexOf("\n", end - 1) + 1;
    const line = text.slice(start, end).trim();
    end = start - 1;
    if (line.length === 0 || line.startsWith("--") || line.startsWith("/*") || line.endsWith("*/")) continue;
    if (line.endsWith(";")) return true;
    return prefix !== null && line.startsWith(prefix);
  }
  return true;
}

/// A prefix dialect's command line: `\dt`, `.tables`, `:schema` (a leading `.` needs a letter after it).
export function isShellCommandLine(line: string, engine: Engine): boolean {
  const prefix = shellDialectOf(engine).prefix;
  if (prefix === null) return false;
  const trimmed = line.trimStart();
  return trimmed.startsWith(prefix) && (prefix !== "." || /^\.[A-Za-z]/.test(trimmed));
}

export interface ScriptBlock {
  /// A run of shell command lines (kept as written) or of ordinary text.
  shell: boolean;
  text: string;
}

// WHAT:  A script cut into its shell command lines and the text between them.
// WHY:   Format reflows SQL; handed `\d users` or `.schema` it would break the line
//        or refuse the whole script.
// HOW:   A line is a command line when it starts with the dialect's prefix and begins
//        a statement (the text above it ends in `;`, or is empty, or is a command line).
export function splitShellBlocks(text: string, engine: Engine): ScriptBlock[] {
  const blocks: ScriptBlock[] = [];
  let pending: string[] = [];
  const flush = () => {
    if (pending.length > 0) blocks.push({ shell: false, text: pending.join("\n") });
    pending = [];
  };
  const startsStatement = (): boolean => {
    for (let i = pending.length - 1; i >= 0; i -= 1) {
      const line = (pending[i] ?? "").trim();
      if (line.length === 0 || line.startsWith("--")) continue;
      return line.endsWith(";");
    }
    return true;
  };
  for (const line of text.split("\n")) {
    if (isShellCommandLine(line, engine) && startsStatement()) {
      flush();
      const last = blocks[blocks.length - 1];
      if (last?.shell === true) last.text += `\n${line.trim()}`;
      else blocks.push({ shell: true, text: line.trim() });
    } else {
      pending.push(line);
    }
  }
  flush();
  return blocks;
}
