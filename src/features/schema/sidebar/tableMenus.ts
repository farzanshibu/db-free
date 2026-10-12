// SOT: sidebar-menu-builders, table-context-menu-entries, schema-header-menu, database-menu, tag-group-menu
import type { DdlProfile, DdlSupport, NamespaceCaps, NamespaceKind, TableInfo, TableTag } from "@/lib/bindings";
import type { SchemaAction } from "../actions";
import type { MenuNode } from "./AnchoredMenu";

// ============================================================================
// WHAT:  The entries of every sidebar menu — table row, schema header, tag group,
//        column, database — as plain data.
// WHY:   Which entries exist, in what order, and which are greyed out (and why) is
//        the whole behaviour of a menu; as pure functions of "what was clicked +
//        what the engine can do" it reads in one place and the component only
//        wires ids to actions.
// HOW:   An entry the engine's DDL profile refuses is shown disabled with the
//        profile's reason as a second line, never hidden: the user sees the
//        feature exists and learns why not. While the profile is still loading,
//        SQL-speaking engines keep the entries enabled and the form reports the
//        engine's own refusal.
// WHERE: src-tauri/src/integrations/ddl/ (DdlProfile), src/features/schema/useDdlProfile.ts
// ============================================================================

/// Whether a schema-form capability is on: the profile's answer when it is known,
/// otherwise "does this engine speak SQL".
export function ddlAllowed(profile: DdlProfile | undefined, speaksSql: boolean, flag: keyof DdlSupport): boolean {
  return profile === undefined ? speaksSql : profile.supported && profile.support[flag];
}

/// The profile's reason an operation is off, when the profile is known.
function refusal(profile: DdlProfile | undefined, allowed: boolean): string | undefined {
  if (profile === undefined || allowed) return undefined;
  return profile.notes[0] ?? `${profile.dialect} cannot do this from the schema forms.`;
}

export function namespaceCaps(profile: DdlProfile | undefined, kind: NamespaceKind): NamespaceCaps | undefined {
  return profile?.namespaces.find((n) => n.kind === kind);
}

export interface TableMenuContext {
  table: TableInfo;
  /// Row object noun for labels ("table" / "collection").
  noun: string;
  speaksSql: boolean;
  columnPreview: boolean;
  expanded: boolean;
  profile: DdlProfile | undefined;
  /// The table is pinned to Home (the entry flips between Pin and Unpin).
  pinned: boolean;
  /// Show the tag entries (the connection has a tag store to write to).
  tagging: boolean;
  /// Every tag of the table's scope.
  scopeTags: readonly TableTag[];
  /// Ids of the tags this table carries.
  taggedWith: ReadonlySet<string>;
}

export function tableMenu(ctx: TableMenuContext): MenuNode[] {
  const { table, profile, speaksSql } = ctx;
  const isView = table.kind === "view";
  const can = (flag: keyof DdlSupport) => ddlAllowed(profile, speaksSql, flag);
  const edit = can("alterTable") && !isView;
  const duplicate = can("duplicateTable") && !isView;
  const empty = can("truncateTable") && !isView;
  const drop = can("dropTable");

  const nodes: MenuNode[] = [
    { kind: "item", id: "open", label: `Open ${isView ? "view" : ctx.noun}`, icon: "table" },
    { kind: "item", id: "open-new", label: "Open in new tab", icon: "plus", hint: "Shift+Click" },
    { kind: "item", id: "columns", label: ctx.expanded ? "Hide columns" : "Show columns", icon: "columns", disabled: !ctx.columnPreview },
    { kind: "item", id: "refresh-columns", label: "Refresh columns", icon: "refresh" },
    { kind: "separator" },
    { kind: "item", id: "select", label: "Open in SQL editor", icon: "terminal", disabled: !speaksSql },
    { kind: "item", id: "count", label: "Count rows", icon: "hash", disabled: !speaksSql },
    { kind: "item", id: "export", label: "Export / import…", icon: "download" },
    { kind: "item", id: "compare-schema", label: "Compare schema…", icon: "git-branch", disabled: !speaksSql },
    { kind: "item", id: "compare-data", label: "Compare data…", icon: "exchange" },
    { kind: "separator" },
    { kind: "item", id: "copy-name", label: "Copy name", icon: "copy" },
    { kind: "item", id: "copy-qualified", label: "Copy qualified name", icon: "copy", disabled: table.schema === null },
    { kind: "item", id: "copy-schema", label: isView ? "Copy view definition" : `Copy ${ctx.noun} schema`, icon: "braces" },
    { kind: "item", id: "refresh", label: "Refresh schema", icon: "refresh" },
    { kind: "separator" },
    { kind: "item", id: ctx.pinned ? "unpin" : "pin", label: ctx.pinned ? "Unpin from Home" : "Pin to Home", icon: "pin" },
    { kind: "item", id: "ai-dashboard", label: "Generate dashboard with AI", icon: "sparkles" },
  ];

  if (ctx.tagging) {
    nodes.push(
      { kind: "separator" },
      {
        kind: "sub",
        label: "Tag table",
        icon: "tag",
        children: [
          ...ctx.scopeTags.map((tag): MenuNode => ({ kind: "check", id: `tag:${tag.id}`, label: tag.name, checked: ctx.taggedWith.has(tag.id), color: tag.color })),
          ...(ctx.scopeTags.length > 0 ? [{ kind: "separator" } satisfies MenuNode] : []),
          { kind: "item", id: "tag-new", label: "New tag…", icon: "plus" },
          { kind: "item", id: "tag-clear", label: "Remove all tags", icon: "x", disabled: ctx.taggedWith.size === 0 },
        ],
      },
    );
  }

  nodes.push(
    { kind: "separator" },
    {
      kind: "item",
      id: "edit",
      label: "Edit table…",
      icon: "pencil",
      disabled: !edit,
      ...(isView ? { note: "A view has no columns to edit here." } : noteOf(refusal(profile, edit))),
    },
    { kind: "item", id: "duplicate", label: "Duplicate table…", icon: "copy", disabled: !duplicate, ...(isView ? {} : noteOf(refusal(profile, duplicate))) },
    { kind: "separator" },
    { kind: "item", id: "empty", label: "Empty table…", icon: "trash", danger: true, disabled: !empty, ...(isView ? {} : noteOf(refusal(profile, empty))) },
    { kind: "item", id: "drop", label: isView ? "Delete view…" : "Delete table…", icon: "trash", danger: true, disabled: !drop, ...noteOf(refusal(profile, drop)) },
  );
  return nodes;
}

function noteOf(note: string | undefined): { note?: string } {
  return note === undefined ? {} : { note };
}

export interface SchemaMenuContext {
  name: string;
  speaksSql: boolean;
  erd: boolean;
  profile: DdlProfile | undefined;
  /// Tags are on screen (Tags mode), so "New tag…" makes sense here.
  tagging: boolean;
}

export function schemaMenu(ctx: SchemaMenuContext): MenuNode[] {
  const caps = namespaceCaps(ctx.profile, "schema");
  const entry = (id: string, label: string, ok: boolean | undefined, danger = false): MenuNode => {
    const allowed = ok === true;
    const note = caps === undefined ? "This engine's schemas cannot be changed from here." : allowed ? undefined : (caps.note ?? undefined);
    return { kind: "item", id, label, icon: danger ? "trash" : "folder", danger, disabled: !allowed, ...noteOf(note) };
  };
  return [
    { kind: "item", id: "only", label: `Show only ${ctx.name}`, icon: "folder" },
    { kind: "item", id: "erd", label: "ER diagram of this schema", icon: "view", disabled: !ctx.erd },
    { kind: "item", id: "compare", label: "Compare schema…", icon: "git-branch", disabled: !ctx.speaksSql },
    { kind: "separator" },
    ...(ctx.tagging ? [{ kind: "item", id: "tag-new", label: "New tag…", icon: "tag" } satisfies MenuNode, { kind: "separator" } satisfies MenuNode] : []),
    { kind: "item", id: "copy", label: "Copy schema name", icon: "copy" },
    { kind: "item", id: "refresh", label: "Refresh schema", icon: "refresh" },
    { kind: "separator" },
    entry("schema-create", "New schema…", caps?.create),
    entry("schema-rename", "Rename schema…", caps?.rename),
    entry("schema-drop", "Delete schema…", caps?.drop, true),
  ];
}

export interface NamespaceMenuContext {
  databases: readonly string[];
  currentDatabase: string;
  /// Schemas of the current database, as the catalog lists them.
  schemas: readonly string[];
  profile: DdlProfile | undefined;
}

// WHAT:  The "⋯" next to the database / schema pickers: create a database or schema,
//        or rename / drop one. Empty when the engine's forms have neither.
// WHY:   You cannot drop or rename the database you are connected to (the server
//        refuses), so the current one is listed disabled with the reason instead
//        of being offered and failing. A schema has no such rule.
export function namespaceMenu(ctx: NamespaceMenuContext): MenuNode[] {
  const database = namespaceCaps(ctx.profile, "database");
  const schema = namespaceCaps(ctx.profile, "schema");
  const nodes: MenuNode[] = [];
  if (database !== undefined) nodes.push(...section("db", "database", ctx.databases, ctx.currentDatabase, database));
  if (database !== undefined && schema !== undefined) nodes.push({ kind: "separator" });
  if (schema !== undefined) nodes.push(...section("ns", "schema", ctx.schemas, null, schema));
  return nodes;
}

function section(prefix: "db" | "ns", noun: string, names: readonly string[], connected: string | null, caps: NamespaceCaps): MenuNode[] {
  const list = (verb: "rename" | "drop"): MenuNode[] =>
    names.map((name): MenuNode => ({
      kind: "item",
      id: `${prefix}-${verb}:${name}`,
      label: name,
      icon: noun === "database" ? "database" : "folder",
      danger: verb === "drop",
      disabled: name === connected,
      ...(name === connected ? { note: "Connected: switch to another database first." } : {}),
    }));
  const verbs = (verb: "rename" | "drop", label: string, allowed: boolean): MenuNode => {
    const danger = verb === "drop";
    return allowed
      ? { kind: "sub", label, icon: danger ? "trash" : "pencil", children: list(verb) }
      : { kind: "sub", label, icon: danger ? "trash" : "pencil", disabled: true, ...noteOf(caps.note ?? undefined), children: [] };
  };
  return [
    { kind: "item", id: `${prefix}-create`, label: `New ${noun}…`, icon: "plus", disabled: !caps.create, ...noteOf(caps.create ? undefined : (caps.note ?? undefined)) },
    verbs("rename", `Rename ${noun}`, caps.rename),
    verbs("drop", `Delete ${noun}`, caps.drop),
  ];
}

export function groupMenu(tag: TableTag): MenuNode[] {
  return [
    { kind: "item", id: "tag-edit", label: `Edit "${tag.name}"…`, icon: "pencil" },
    { kind: "item", id: "tag-new", label: "New tag…", icon: "plus" },
    { kind: "separator" },
    { kind: "item", id: "tag-delete", label: "Delete tag…", icon: "trash", danger: true },
  ];
}

export function columnMenu(dataType: string): MenuNode[] {
  return [
    { kind: "item", id: "name", label: "Copy column name", icon: "copy" },
    { kind: "item", id: "type", label: `Copy type (${dataType})`, icon: "copy" },
  ];
}

// ---- menu id -> schema form -----------------------------------------------------

/// The form a table menu entry opens, if it opens one.
export function tableAction(id: string, table: TableInfo): SchemaAction | null {
  const ref = { schema: table.schema, name: table.name };
  switch (id) {
    case "edit":
      return { kind: "edit-table", table: ref };
    case "duplicate":
      return { kind: "duplicate-table", table: ref };
    case "empty":
      return { kind: "remove-table", table: ref, tableKind: table.kind, action: "truncate" };
    case "drop":
      return { kind: "remove-table", table: ref, tableKind: table.kind, action: "drop" };
    default:
      return null;
  }
}

/// The form a schema header menu entry opens, if it opens one.
export function schemaAction(id: string, schema: string): SchemaAction | null {
  switch (id) {
    case "schema-create":
      return { kind: "create-namespace", namespace: "schema" };
    case "schema-rename":
      return { kind: "rename-namespace", namespace: "schema", name: schema };
    case "schema-drop":
      return { kind: "drop-namespace", namespace: "schema", name: schema };
    default:
      return null;
  }
}

// WHAT:  The form a "⋯" namespace menu entry opens. Ids are `db-create`,
//        `db-rename:<name>`, `db-drop:<name>` and the same with `ns-` for schemas.
export function namespaceAction(id: string): SchemaAction | null {
  const sep = id.indexOf(":");
  const head = sep < 0 ? id : id.slice(0, sep);
  const name = sep < 0 ? "" : id.slice(sep + 1);
  const namespace: NamespaceKind | null = head.startsWith("db-") ? "database" : head.startsWith("ns-") ? "schema" : null;
  if (namespace === null) return null;
  switch (head.slice(3)) {
    case "create":
      return { kind: "create-namespace", namespace };
    case "rename":
      return { kind: "rename-namespace", namespace, name };
    case "drop":
      return { kind: "drop-namespace", namespace, name };
    default:
      return null;
  }
}
