// SOT: ddl-column-locks, ddl-form-validation, ddl-foreign-key-display
import type { DdlProfile, FkAction, TableRef } from "@/lib/bindings";
import { FK_ACTIONS, IDENT_RE, hasColumns, nounOf, paramsOf, parseEnumText, qualifiedLabel, type ColumnDraft, type ForeignKeyDraft, type TableDraft } from "./ddlForm";

// WHAT:  The rules a schema form applies to its draft: which edits an engine
//        allows on an existing column, which fields are invalid before the server
//        is asked, and how a foreign key reads in a list.
// WHY:   Kept apart from the draft's shape and conversion (`ddlForm.ts`) so each
//        module stays small enough to read in one sitting; both are pure.
// HOW:   Locks come from the engine's `DdlProfile.support` flags, so a disabled
//        control and the server's refusal always agree. Validation never re-checks
//        an existing name, only names typed this session.
// WHERE: src/features/schema/ddlForm.ts, src/features/schema/forms/*

// ---- what the engine lets an existing column do -----------------------------------

/// Why a control is off ("" never): null means the control is live.
export interface ColumnLocks {
  name: string | null;
  type: string | null;
  nullable: string | null;
  defaultValue: string | null;
  unique: string | null;
  primaryKey: string | null;
  autoIncrement: string | null;
  remove: string | null;
}

const OPEN_LOCKS: ColumnLocks = { name: null, type: null, nullable: null, defaultValue: null, unique: null, primaryKey: null, autoIncrement: null, remove: null };

// WHAT:  For an existing column, the reason each edit is unavailable on this
//        engine (null = available).
// WHY:   Greying a control without saying why reads as a bug. An engine that
//        rebuilds the table (SQLite) can do any of these, so it locks nothing.
export function columnLocks(profile: DdlProfile, column: ColumnDraft): ColumnLocks {
  if (column.originalName === null || profile.support.rebuilds) return OPEN_LOCKS;
  const s = profile.support;
  const no = (what: string): string => `${profile.dialect} cannot ${what} on an existing column.`;
  return {
    name: s.renameColumn ? null : no("rename it"),
    type: s.changeType ? null : no("change the type"),
    nullable: s.changeNullability ? null : no("change nullability"),
    defaultValue: s.changeDefault ? null : no("change the default"),
    unique: s.changeUnique ? null : no("change uniqueness"),
    primaryKey: s.changePrimaryKey ? null : no("change the primary key"),
    autoIncrement: "Auto-increment can only be set when the column is created.",
    remove: s.dropColumn ? null : `${profile.dialect} cannot drop a column.`,
  };
}

export function renameTableLock(profile: DdlProfile): string | null {
  return profile.support.renameTable || profile.support.rebuilds ? null : `${profile.dialect} cannot rename a ${nounOf(profile)}.`;
}

export function foreignKeyLock(profile: DdlProfile): string | null {
  return profile.support.alterForeignKeys || profile.support.rebuilds ? null : `${profile.dialect} cannot add or drop a foreign key on an existing ${nounOf(profile)}.`;
}

// ---- validation -------------------------------------------------------------------

export interface DraftIssues {
  table: string | null;
  /// By column `key`.
  columns: Record<string, string>;
  /// By foreign key `key`.
  foreignKeys: Record<string, string>;
  general: string[];
  count: number;
}

function nameProblem(kind: string, name: string): string | null {
  if (name.trim() === "") return `${kind} name is required`;
  if (!IDENT_RE.test(name)) return `${kind} name must start with a letter or underscore and contain only letters, numbers, and underscores`;
  return null;
}

// WHAT:  Field-level problems in a draft, found without asking the server.
// WHY:   The live preview only runs for a complete form, so a half-typed name does
//        not flash a builder error on every keystroke. The builders still validate
//        (the server is authoritative); this keeps the common mistakes local.
// HOW:   An existing name is never re-checked: a table called "Order Items" must
//        stay editable. Only names the user typed this session are held to IDENT_RE.
export function validateDraft(profile: DdlProfile, draft: TableDraft, originalTable: TableRef | null): DraftIssues {
  const issues: DraftIssues = { table: null, columns: {}, foreignKeys: {}, general: [], count: 0 };
  const noun = profile.noun;
  if (originalTable?.name !== draft.name) issues.table = nameProblem(noun, draft.name);

  if (hasColumns(profile)) {
    const live = draft.columns.filter((c) => !c.removed);
    if (live.length === 0) issues.general.push("At least one column is required.");
    const seen = new Set<string>();
    for (const c of live) {
      const lower = c.name.toLowerCase();
      let problem: string | null = null;
      if (c.originalName === null || c.name !== c.originalName) problem = nameProblem("Column", c.name);
      if (problem === null && seen.has(lower)) problem = "Column name must be unique";
      seen.add(lower);
      if (problem === null && c.dataType.trim() === "") problem = "Choose a type";
      const params = paramsOf(profile, c);
      if (problem === null && params === "values" && parseEnumText(c.enumText).length === 0) problem = "List the allowed values, separated by commas";
      if (problem === null && params === "length" && c.length !== null && (!Number.isInteger(c.length) || c.length < 1)) problem = "Length must be a positive whole number";
      if (problem === null && params === "precision_scale" && c.precision !== null && c.scale !== null && c.scale > c.precision) problem = "Scale cannot exceed precision";
      if (problem === null && c.defaultKind === "custom" && c.defaultExpression.trim() === "") problem = "Write the default expression";
      if (problem !== null) issues.columns[c.key] = problem;
    }
    if (profile.support.tableEngine && (draft.engine === null || draft.engine === "")) issues.general.push("Choose a table engine.");

    const names = new Set(live.map((c) => c.name));
    // Live keys are the server's; only the ones typed this session are checked.
    for (const fk of draft.foreignKeys.filter((f) => !f.removed && f.originalName === null)) {
      let problem: string | null = null;
      if (fk.columns.length === 0) problem = "Pick at least one column";
      else if (fk.columns.some((c) => !names.has(c))) problem = "A chosen column is not in this table";
      else if (!fk.refSelf && fk.refTable === "") problem = "Pick the referenced table";
      else if (fk.refColumns.length !== fk.columns.length) problem = "Pick as many referenced columns as local columns";
      if (problem !== null) issues.foreignKeys[fk.key] = problem;
    }
  }

  issues.count = (issues.table === null ? 0 : 1) + Object.keys(issues.columns).length + Object.keys(issues.foreignKeys).length + issues.general.length;
  return issues;
}

/// A new schema / database / copy name: required, an identifier, not already taken.
export function validateNewName(kind: string, name: string, taken: readonly string[]): string | null {
  const problem = nameProblem(kind, name);
  if (problem !== null) return problem;
  const lower = name.toLowerCase();
  return taken.some((t) => t.toLowerCase() === lower) ? `A ${kind.toLowerCase()} named "${name}" already exists` : null;
}

// ---- foreign key display ------------------------------------------------------------

export function describeForeignKey(fk: ForeignKeyDraft, own: TableRef): string {
  const target = fk.refSelf ? own : { schema: fk.refSchema, name: fk.refTable };
  const actions = [fk.onDelete !== "no_action" ? `ON DELETE ${actionLabel(fk.onDelete)}` : null, fk.onUpdate !== "no_action" ? `ON UPDATE ${actionLabel(fk.onUpdate)}` : null].filter((a) => a !== null);
  const tail = actions.length > 0 ? ` · ${actions.join(" · ")}` : "";
  return `(${fk.columns.join(", ")}) → ${qualifiedLabel(target)} (${fk.refColumns.join(", ")})${tail}`;
}

export function actionLabel(action: FkAction): string {
  return FK_ACTIONS.find((a) => a.value === action)?.label ?? action;
}
