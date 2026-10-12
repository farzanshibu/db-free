// SOT: table-tags-store, tag-state-cache, optimistic-tag-updates, tag-reload
import { useEffect } from "react";
import { create } from "zustand";
import type { TableTag, TableTagInput, TagState } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";

// ============================================================================
// WHAT:  The sidebar's table tags for one connection + database, cached on the
//        client and kept in step with the app's local store.
// WHY:   Tags drive what the list draws on every render; a round trip per toggle
//        would make multi-tagging a table feel like a form. Tags are app
//        metadata (never the user's database), so a stale cache costs a wrong
//        dot, not wrong data — but failed writes are still rolled back.
// HOW:   One `TagState` per `${connectionId}\0${database}`. Cheap writes
//        (tag a table, fold a group, reorder) update the cache first and undo it
//        with a toast if the command fails; writes whose result matters (save,
//        delete, move) wait for the server and then reload. `reload` is also the
//        hook other features call after they rename or drop a table.
// WHERE: src-tauri/src/commands/tags.rs, src/features/shell/TablesPanel.tsx
// ============================================================================

export function tagScopeKey(connectionId: string, database: string): string {
  return `${connectionId}\u0000${database}`;
}

interface TagStore {
  states: Record<string, TagState>;
  load: (connectionId: string, database: string) => Promise<void>;
  /// Same as `load`; the name other features reach for after a table changed.
  reload: (connectionId: string, database: string) => Promise<void>;
  /// Creates (`input.id` null) or edits a tag. Throws the server's refusal
  /// (duplicate name…) so the dialog can print it under the field.
  saveTag: (input: TableTagInput) => Promise<TableTag>;
  deleteTag: (connectionId: string, database: string, tagId: string) => Promise<void>;
  reorder: (connectionId: string, database: string, schema: string, ids: readonly string[]) => Promise<void>;
  tagTable: (connectionId: string, database: string, tagId: string, table: string, tagged: boolean) => Promise<void>;
  /// Takes every tag of the scope off one table.
  untagTable: (connectionId: string, database: string, scope: string, table: string) => Promise<void>;
  /// `tagId` null is the scope's fixed "Untagged" group.
  setCollapsed: (connectionId: string, database: string, schema: string, tagId: string | null, collapsed: boolean) => Promise<void>;
  /// Follows a table through a rename (`to` set) or forgets it after a drop.
  moveTable: (connectionId: string, database: string, schema: string, from: string, to: string | null) => Promise<void>;
}

const EMPTY: TagState = { tags: [], links: [], untaggedCollapsed: [] };

/// Latest request per scope, so a slow answer cannot overwrite a newer one.
const latest = new Map<string, number>();

type Raw = Parameters<typeof normalizeError>[0];

function fail(raw: Raw): void {
  useWorkspace.getState().showError(normalizeError(raw));
}

export const useTableTags = create<TagStore>()((set, get) => {
  const read = (key: string): TagState => get().states[key] ?? EMPTY;
  const write = (key: string, state: TagState): void => { set((s) => ({ states: { ...s.states, [key]: state } })); };

  // Applies `next` now and restores the previous state if `call` fails.
  const optimistic = async (key: string, next: (state: TagState) => TagState, call: () => Promise<null>): Promise<void> => {
    const before = read(key);
    write(key, next(before));
    try {
      await call();
    } catch (raw) {
      write(key, before);
      fail(raw);
    }
  };

  const load = async (connectionId: string, database: string): Promise<void> => {
    const key = tagScopeKey(connectionId, database);
    const ticket = (latest.get(key) ?? 0) + 1;
    latest.set(key, ticket);
    try {
      const state = await ipc("list_table_tags", { connectionId, database });
      if (latest.get(key) === ticket) write(key, state);
    } catch (raw) {
      fail(raw);
    }
  };

  return {
    states: {},
    load,
    reload: load,

    saveTag: async (input) => {
      const saved = await ipc("save_table_tag", { tag: input });
      await load(input.connectionId, input.database);
      return saved;
    },

    deleteTag: async (connectionId, database, tagId) => {
      try {
        await ipc("delete_table_tag", { id: tagId });
      } catch (raw) {
        fail(raw);
      }
      await load(connectionId, database);
    },

    reorder: async (connectionId, database, schema, ids) => {
      const key = tagScopeKey(connectionId, database);
      await optimistic(
        key,
        (state) => ({
          ...state,
          tags: state.tags.map((tag) => {
            const position = ids.indexOf(tag.id);
            return tag.schema === schema && position >= 0 ? { ...tag, position } : tag;
          }),
        }),
        () => ipc("reorder_table_tags", { connectionId, database, schema, ids: [...ids] }),
      );
    },

    tagTable: async (connectionId, database, tagId, table, tagged) => {
      const key = tagScopeKey(connectionId, database);
      await optimistic(
        key,
        (state) => {
          const others = state.links.filter((l) => !(l.tagId === tagId && l.table === table));
          return { ...state, links: tagged ? [...others, { tagId, table }] : others };
        },
        () => ipc("tag_table", { tagId, table, tagged }),
      );
    },

    untagTable: async (connectionId, database, scope, table) => {
      const key = tagScopeKey(connectionId, database);
      const ids = read(key).tags.filter((t) => t.schema === scope && read(key).links.some((l) => l.tagId === t.id && l.table === table)).map((t) => t.id);
      if (ids.length === 0) return;
      await optimistic(
        key,
        (state) => ({ ...state, links: state.links.filter((l) => !(ids.includes(l.tagId) && l.table === table)) }),
        async () => {
          for (const tagId of ids) await ipc("tag_table", { tagId, table, tagged: false });
          return null;
        },
      );
    },

    setCollapsed: async (connectionId, database, schema, tagId, collapsed) => {
      const key = tagScopeKey(connectionId, database);
      await optimistic(
        key,
        (state) =>
          tagId === null
            ? { ...state, untaggedCollapsed: collapsed ? [...state.untaggedCollapsed.filter((s) => s !== schema), schema] : state.untaggedCollapsed.filter((s) => s !== schema) }
            : { ...state, tags: state.tags.map((tag) => (tag.id === tagId ? { ...tag, collapsed } : tag)) },
        () => ipc("set_tag_collapsed", { connectionId, database, schema, tagId, collapsed }),
      );
    },

    moveTable: async (connectionId, database, schema, from, to) => {
      try {
        await ipc("move_tagged_table", { connectionId, database, schema, from, to });
      } catch (raw) {
        fail(raw);
      }
      await load(connectionId, database);
    },
  };
});

// WHAT:  The cached tags for a connection + database, loaded the first time they
//        are asked for and again whenever `refreshOn` changes (the catalog object:
//        a renamed or dropped table may have changed which links still apply).
export function useTagState(connectionId: string | null, database: string, refreshOn?: object): TagState | undefined {
  const key = connectionId === null ? null : tagScopeKey(connectionId, database);
  const state = useTableTags((s) => (key === null ? undefined : s.states[key]));
  const load = useTableTags((s) => s.load);
  useEffect(() => {
    if (connectionId !== null) void load(connectionId, database);
  }, [connectionId, database, load, refreshOn]);
  return state;
}
