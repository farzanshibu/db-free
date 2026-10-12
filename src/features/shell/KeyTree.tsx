// SOT: key-tree, redis-key-grouping, key-namespace-folders, key-search, key-value-preview-toggle, key-scan-progress
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RedisKeyEntry, TableInfo } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { isRedisEngine } from "@/lib/redisCommands";
import { useWorkspace } from "@/stores/workspace";
import { formatCount } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { IconButton } from "@/components/global/Button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { CreateKeyDialog } from "@/features/keys/CreateKeyDialog";
import { keyRef } from "@/features/keys/keyCommands";
import { KEY_TYPE_ICON } from "@/features/keys/keyFormat";
import { PAGE_KEYS, useKeyBrowser } from "@/features/keys/keyBrowser";
import { buildTree, CHILDREN_STEP, detectSeparator, sortedChildren, type KeyItem, type KeyNode } from "@/features/keys/keyTree";

// WHAT:  The sidebar's key list. Redis keys are scanned from the server with a
//        cursor — however many there are — grouped into folders by their
//        separator, drawn with a type icon each, and searched on the server.
//        Other key-value engines show the keys their catalog lists.
// WHY:   A catalog capped at a few thousand keys silently hid the rest of a real
//        keyspace; a client-side filter of that capped list found nothing in
//        it. The tree now pages the whole keyspace and asks the server to match.
// HOW:   `useKeyBrowser` (src/features/keys/keyBrowser.ts) owns the scan; this
//        file draws it. A folder shows 100 children and "Show more" steps by
//        100. The search box queries `SCAN MATCH *text*` after a pause and lists
//        the matches in place of the tree.
// WHERE: src/features/shell/TablesPanel.tsx (mounts it), src-tauri/src/integrations/redis.rs
export function KeyTree({ connectionId, keys }: { connectionId: string; keys: TableInfo[] }) {
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine);
  if (engine !== undefined && isRedisEngine(engine)) return <RedisKeyTree connectionId={connectionId} />;
  return <CatalogKeyTree connectionId={connectionId} keys={keys} />;
}

// ---------------------------------------------------------------------------
// Engines whose keys come from the catalog (Memcached, RocksDB)
// ---------------------------------------------------------------------------

function CatalogKeyTree({ connectionId, keys }: { connectionId: string; keys: TableInfo[] }) {
  const items = useMemo<KeyItem[]>(() => keys.map((k) => ({ name: k.name, kind: null, preview: null })), [keys]);
  return <Tree connectionId={connectionId} items={items} previews={false} />;
}

// ---------------------------------------------------------------------------
// Redis
// ---------------------------------------------------------------------------

const SEARCH_PAUSE_MS = 250;
const SEARCH_RESULTS = 100;

interface Search {
  text: string;
  results: readonly RedisKeyEntry[];
  cursor: string;
  done: boolean;
  loading: boolean;
  error: string | null;
}

const NO_SEARCH: Search = { text: "", results: [], cursor: "0", done: true, loading: false, error: null };

function RedisKeyTree({ connectionId }: { connectionId: string }) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const browser = useKeyBrowser((s) => s.browsers[connectionId]);
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const openTable = useWorkspace((s) => s.openTable);
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState<Search>(NO_SEARCH);
  const generation = useRef(0);

  // A new catalog (connect, the sidebar's refresh) means the keyspace may have changed.
  useEffect(() => {
    void useKeyBrowser.getState().load(connectionId, catalog ?? null);
  }, [connectionId, catalog]);

  const previews = browser?.previews ?? false;
  // WHAT:  One page of matches for `text` from `cursor`; `append` continues a search.
  const run = useCallback(
    async (text: string, cursor: string, append: boolean) => {
      const mine = ++generation.current;
      setSearch((s) => ({ ...s, text, loading: true, error: null, ...(append ? {} : { results: [], cursor: "0", done: false }) }));
      try {
        const page = await ipc("scan_keys", { connectionId, scan: { search: text, cursor, count: PAGE_KEYS, limit: SEARCH_RESULTS, previews } });
        if (mine !== generation.current) return;
        setSearch((s) => ({
          ...s,
          results: append ? [...s.results, ...page.keys.filter((k) => !s.results.some((r) => r.name === k.name))] : page.keys,
          cursor: page.cursor,
          done: page.done,
          loading: false,
        }));
      } catch (raw) {
        if (mine === generation.current) setSearch((s) => ({ ...s, loading: false, error: normalizeError(raw).message }));
      }
    },
    [connectionId, previews],
  );

  // The search runs a moment after typing stops.
  useEffect(() => {
    const text = query.trim();
    if (text === "") {
      // Nothing to look for: a search still in flight is abandoned (its answer would be stale).
      generation.current += 1;
      return;
    }
    const id = window.setTimeout(() => {
      void run(text, "0", false);
    }, SEARCH_PAUSE_MS);
    return () => {
      window.clearTimeout(id);
    };
  }, [query, run]);

  const items = useMemo<KeyItem[]>(() => (browser?.keys ?? []).map((k) => ({ name: k.name, kind: k.kind, preview: k.preview })), [browser?.keys]);
  const searching = query.trim() !== "";
  // Until the pause after typing ends the previous search is not this query's answer.
  const shown: Search = search.text === query.trim() ? search : { ...NO_SEARCH, text: query.trim(), done: false, loading: true };

  return (
    <div className="flex min-h-0 flex-col">
      <div className="sticky top-0 z-10 flex items-center gap-1 bg-surface-secondary/60 px-1.5 pb-1 backdrop-blur">
        <SearchInput value={query} onChange={setQuery} aria-label="Search keys" placeholder="Search keys on the server…" className="glass-input h-6 min-w-0 flex-1 rounded-md text-xs" />
        <IconButton
          icon={previews ? "eye-off" : "eye"}
          label={previews ? "Hide values" : "Show values"}
          active={previews}
          onClick={() => {
            void useKeyBrowser.getState().setPreviews(connectionId, !previews);
          }}
        />
        <IconButton
          icon="refresh"
          label="Rescan keys"
          onClick={() => {
            void useKeyBrowser.getState().refresh(connectionId);
          }}
        />
        <CreateKeyDialog
          connectionId={connectionId}
          trigger={
            <Button variant="ghost" size="icon-sm" aria-label="New key" disabled={readOnly} className="rounded-lg liquid-hover">
              <Icon name="plus" size={13} />
            </Button>
          }
          onCreated={(name, kind) => {
            useKeyBrowser.getState().add(connectionId, name, kind);
            openTable(connectionId, keyRef(name));
          }}
        />
      </div>
      {searching ? (
        <SearchResults connectionId={connectionId} search={shown} previews={previews} onMore={() => void run(shown.text, shown.cursor, true)} />
      ) : browser === undefined ? (
        <div className="flex justify-center p-2.5">
          <Spinner size="sm" />
        </div>
      ) : (
        <>
          <ScanStatus connectionId={connectionId} loaded={browser.keys.length} total={browser.total} scanning={browser.scanning} done={browser.done} error={browser.error} />
          {browser.keys.length === 0 && !browser.scanning && browser.error === null ? (
            <p className="px-2.5 py-2 text-xs text-muted">No keys in this database yet.</p>
          ) : (
            <Tree connectionId={connectionId} items={items} previews={previews} />
          )}
        </>
      )}
    </div>
  );
}

function ScanStatus({ connectionId, loaded, total, scanning, done, error }: { connectionId: string; loaded: number; total: number | null; scanning: boolean; done: boolean; error: string | null }) {
  return (
    <div className="flex items-center gap-1.5 px-2.5 pb-1 text-[11px] text-muted">
      {scanning ? <Spinner size="sm" /> : null}
      <span>
        {formatCount(loaded)}
        {total !== null && total > loaded ? ` of ~${formatCount(total)}` : ""} {loaded === 1 ? "key" : "keys"}
        {scanning ? " · scanning…" : ""}
      </span>
      {!done && !scanning && error === null ? (
        <Button
          variant="link"
          size="xs"
          className="ml-auto h-5 px-1"
          onClick={() => {
            void useKeyBrowser.getState().more(connectionId);
          }}
        >
          Load more
        </Button>
      ) : null}
      {error !== null ? (
        <span className="truncate text-danger" title={error}>
          {error}
        </span>
      ) : null}
      {error !== null ? (
        <Button
          variant="link"
          size="xs"
          className="ml-auto h-5 px-1"
          onClick={() => {
            void useKeyBrowser.getState().more(connectionId);
          }}
        >
          Retry
        </Button>
      ) : null}
    </div>
  );
}

function SearchResults({ connectionId, search, previews, onMore }: { connectionId: string; search: Search; previews: boolean; onMore: () => void }) {
  const openTable = useWorkspace((s) => s.openTable);
  const activeTabId = useWorkspace((s) => s.activeTabId);
  return (
    <div>
      <div className="flex items-center gap-1.5 px-2.5 pb-1 text-[11px] text-muted">
        {search.loading ? <Spinner size="sm" /> : null}
        <span>
          {search.error ?? (search.loading && search.results.length === 0 ? `Searching for "${search.text}"…` : `${formatCount(search.results.length)} match${search.results.length === 1 ? "" : "es"}`)}
          {!search.done && !search.loading && search.error === null ? " so far" : ""}
        </span>
      </div>
      <ul>
        {search.results.map((result) => (
          <li key={result.name}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                openTable(connectionId, keyRef(result.name));
              }}
              title={result.name}
              className={cn(
                "flex h-6 w-full items-center justify-start gap-2 rounded-none px-2.5 text-left text-xs",
                activeTabId === `table:${connectionId}:${result.name}` ? "bg-surface-tertiary text-foreground" : "text-muted hover:bg-surface-secondary hover:text-foreground",
              )}
            >
              <Icon name={KEY_TYPE_ICON[result.kind]} size={13} className="shrink-0 opacity-70" />
              <span className="min-w-0 flex-1 truncate">{result.name}</span>
              {previews && result.preview !== null ? <span className="max-w-[40%] shrink-0 truncate font-mono text-[10px] text-muted/70">{result.preview === "" ? '""' : result.preview}</span> : null}
            </Button>
          </li>
        ))}
      </ul>
      {!search.done && !search.loading && search.error === null ? (
        <Button variant="link" size="xs" className="mx-3 my-1 h-6 px-0" onClick={onMore}>
          Search further
        </Button>
      ) : null}
      {search.done && !search.loading && search.error === null && search.results.length === 0 ? <p className="px-2.5 py-1.5 text-xs text-muted">No key contains "{search.text}".</p> : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The folder tree
// ---------------------------------------------------------------------------

function Tree({ connectionId, items, previews }: { connectionId: string; items: readonly KeyItem[]; previews: boolean }) {
  const separator = useMemo(() => detectSeparator(items), [items]);
  const root = useMemo(() => buildTree(items, separator), [items, separator]);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const [limits, setLimits] = useState<ReadonlyMap<string, number>>(new Map());

  const toggle = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };
  const showMore = (path: string) => {
    setLimits((current) => new Map(current).set(path, (current.get(path) ?? CHILDREN_STEP) + CHILDREN_STEP));
  };

  return (
    <ul>
      <Children parent={root} depth={0} connectionId={connectionId} previews={previews} expanded={expanded} limits={limits} onToggle={toggle} onMore={showMore} />
    </ul>
  );
}

interface ChildrenProps {
  parent: KeyNode;
  depth: number;
  connectionId: string;
  previews: boolean;
  expanded: ReadonlySet<string>;
  limits: ReadonlyMap<string, number>;
  onToggle: (path: string) => void;
  onMore: (path: string) => void;
}

type RowProps = Omit<ChildrenProps, "parent"> & { node: KeyNode };

function Children({ parent, depth, ...rest }: ChildrenProps) {
  const all = sortedChildren(parent);
  const limit = rest.limits.get(parent.path) ?? CHILDREN_STEP;
  const hidden = all.length - limit;
  return (
    <>
      {all.slice(0, limit).map((child) => (
        <Row key={child.path} node={child} depth={depth} {...rest} />
      ))}
      {hidden > 0 ? (
        <li>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              rest.onMore(parent.path);
            }}
            style={{ paddingLeft: 8 + depth * 16 + 20 }}
            className="flex h-6 w-full items-center justify-start gap-1.5 rounded-none pr-2 text-xs text-accent hover:bg-surface-secondary"
          >
            <Icon name="chevron-down" size={11} />
            Show {formatCount(Math.min(hidden, CHILDREN_STEP))} more ({formatCount(hidden)} remaining)
          </Button>
        </li>
      ) : null}
    </>
  );
}

function Row({ node, depth, connectionId, previews, expanded, limits, onToggle, onMore }: RowProps) {
  const openTable = useWorkspace((s) => s.openTable);
  const activeTabId = useWorkspace((s) => s.activeTabId);
  const hasChildren = node.children.size > 0;
  const open = expanded.has(node.path);
  const item = node.item;
  const active = item !== null && activeTabId === `table:${connectionId}:${item.name}`;
  const icon = hasChildren && item === null ? "folder" : item !== null && item.kind !== null ? KEY_TYPE_ICON[item.kind] : "key";

  return (
    <li>
      <div
        className={cn("group flex h-6 cursor-default items-center gap-1 pr-2 text-xs", active ? "bg-surface-tertiary text-foreground" : "text-muted hover:bg-surface-secondary hover:text-foreground")}
        style={{ paddingLeft: 8 + depth * 16 }}
        title={node.path}
      >
        {hasChildren ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              onToggle(node.path);
            }}
            aria-label={open ? "Collapse" : "Expand"}
            aria-expanded={open}
            className="flex size-5 min-w-5 items-center justify-center rounded-sm p-0 text-muted hover:text-foreground"
          >
            <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
          </Button>
        ) : (
          <span className="size-5 shrink-0" />
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            if (item !== null) openTable(connectionId, keyRef(item.name));
            else onToggle(node.path);
          }}
          className="flex h-auto min-w-0 flex-1 items-center justify-start gap-2 bg-transparent p-0 text-left hover:bg-transparent"
        >
          <Icon name={icon} size={13} className={cn("shrink-0", item !== null ? "text-accent" : "opacity-70")} />
          <span className="truncate">{node.name.length > 0 ? node.name : "(empty)"}</span>
          {previews && item !== null && item.preview !== null ? <span className="ml-auto max-w-[45%] shrink-0 truncate font-mono text-[10px] text-muted/70">{item.preview === "" ? '""' : item.preview}</span> : null}
        </Button>
        {hasChildren ? (
          <Badge size="sm" variant="soft" className="ml-auto shrink-0 font-mono text-[9px]">
            {formatCount(node.count)}
          </Badge>
        ) : null}
      </div>
      {open && hasChildren ? (
        <ul>
          <Children parent={node} depth={depth + 1} connectionId={connectionId} previews={previews} expanded={expanded} limits={limits} onToggle={onToggle} onMore={onMore} />
        </ul>
      ) : null}
    </li>
  );
}
