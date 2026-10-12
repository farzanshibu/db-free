// SOT: key-tab, redis-key-viewer, key-value-editor, key-metadata, key-rename, key-delete
import { useCallback, useEffect, useState } from "react";
import type { RedisKeyInfo, RedisKeyType, TablePage, TableRef } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { DENSITIES, formatCount } from "@/lib/format";
import { isRedisEngine, redisLine } from "@/lib/redisCommands";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { DataGrid } from "@/features/grid/DataGrid";
import { TableTab } from "@/features/grid/TableTab";
import { EmptyState } from "@/components/global/EmptyState";
import { IconButton } from "@/components/global/Button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { CollectionEditor } from "./CollectionEditor";
import { ConfirmPopover } from "./ConfirmPopover";
import { DocumentEditor } from "./DocumentEditor";
import { keyRef, replyInt, runRedis } from "./keyCommands";
import { formatBytes, formatIdle, KEY_TYPE_ICON, KEY_TYPE_LABEL } from "./keyFormat";
import { useKeyBrowser } from "./keyBrowser";
import { TtlEditor } from "./TtlEditor";

// WHAT:  The key viewer: type, name, size, memory, encoding, idle time, expiry,
//        and the editor that fits the type — a code editor for a string or a
//        RedisJSON document, row editors for a hash, list, set or sorted set, a
//        grid for a stream. Rename, delete and reload sit beside the name.
// WHY:   DB Manager-style key work: open a key, see what it is, change it.
// HOW:   Metadata comes from one `key_info` call (type, TTL, MEMORY USAGE,
//        OBJECT ENCODING / IDLETIME and the size, in one pipeline); values are
//        read through `fetch_table_page`; every write is a command through
//        `execute_query`, so the read-only lock and the history apply.
//        Engines that only look like key-value stores (Memcached, RocksDB) get
//        the plain table view: Redis commands mean nothing to them.
// WHERE: src-tauri/src/integrations/redis.rs (key_info, fetch_page)
export function KeyTab({ connectionId, table }: { connectionId: string; table: TableRef }) {
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine);
  if (engine === undefined || !isRedisEngine(engine)) return <TableTab connectionId={connectionId} table={table} />;
  return <RedisKeyView connectionId={connectionId} table={table} />;
}

type Status = { state: "loading" } | { state: "ready"; info: RedisKeyInfo; at: number } | { state: "missing" } | { state: "failed"; message: string };

const SIZE_NOUN: Record<RedisKeyType, string> = {
  string: "characters",
  hash: "fields",
  list: "items",
  set: "members",
  zset: "members",
  stream: "entries",
  json: "",
  other: "",
};

function RedisKeyView({ connectionId, table }: { connectionId: string; table: TableRef }) {
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const closeTab = useWorkspace((s) => s.closeTab);
  const openTable = useWorkspace((s) => s.openTable);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [status, setStatus] = useState<Status>({ state: "loading" });
  // `revision` remounts the editor (Reload); `infoVersion` only re-reads the figures.
  const [revision, setRevision] = useState(0);
  const [infoVersion, setInfoVersion] = useState(0);
  const key = table.name;
  const tabId = `table:${connectionId}:${tableKey(table)}`;
  const reloadInfo = useCallback(() => {
    setInfoVersion((v) => v + 1);
  }, []);

  useEffect(() => {
    const token = { stale: false };
    void (async () => {
      try {
        const info = await ipc("key_info", { connectionId, key });
        if (!token.stale) setStatus({ state: "ready", info, at: Date.now() });
      } catch (raw) {
        if (token.stale) return;
        const error = normalizeError(raw);
        setStatus(error.kind === "not_found" ? { state: "missing" } : { state: "failed", message: error.message });
      }
    })();
    return () => {
      token.stale = true;
    };
  }, [connectionId, key, revision, infoVersion]);

  const remove = async () => {
    try {
      await runRedis(connectionId, [redisLine("DEL", key)], true);
      useKeyBrowser.getState().remove(connectionId, key);
      showInfo(`Deleted ${key}.`);
      closeTab(tabId);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const info = status.state === "ready" ? status.info : null;
  const kind = info?.kind;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border bg-surface">
        {kind !== undefined ? <Icon name={KEY_TYPE_ICON[kind]} size={14} className="shrink-0 text-accent" /> : null}
        {info !== null ? (
          <Badge size="sm" variant="soft" title={info.typeName}>
            {KEY_TYPE_LABEL[info.kind]}
          </Badge>
        ) : null}
        <span className="selectable min-w-0 truncate font-mono text-xs text-foreground" title={key}>
          {key}
        </span>
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {info !== null ? <Metadata info={info} /> : null}
          {status.state === "ready" ? (
            <TtlEditor
              connectionId={connectionId}
              keyName={key}
              expiresAt={status.info.ttlMs >= 0 ? status.at + status.info.ttlMs : null}
              readOnly={readOnly}
              onChanged={reloadInfo}
            />
          ) : null}
          <RenameKey
            connectionId={connectionId}
            keyName={key}
            disabled={readOnly || status.state !== "ready"}
            onRenamed={(next) => {
              useKeyBrowser.getState().remove(connectionId, key);
              if (kind !== undefined) useKeyBrowser.getState().add(connectionId, next, kind);
              closeTab(tabId);
              openTable(connectionId, keyRef(next));
            }}
          />
          <IconButton
            icon="refresh"
            label="Reload key"
            onClick={() => {
              setStatus({ state: "loading" });
              setRevision((r) => r + 1);
            }}
          />
          <ConfirmPopover
            title={`Delete "${key}"?`}
            body="The key and its value are removed from the server. This cannot be undone."
            confirmLabel="Delete key"
            onConfirm={() => void remove()}
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label="Delete key" disabled={readOnly || status.state !== "ready"}>
                <Icon name="trash" size={14} />
              </Button>
            }
          />
        </div>
      </div>
      <div className="min-h-0 flex-1">
        {status.state === "loading" ? (
          <div className="flex h-full items-center justify-center">
            <Spinner size="sm" />
          </div>
        ) : status.state === "missing" ? (
          <EmptyState icon="key" title="This key no longer exists" body="It was deleted or it expired. Reload to check again, or close this tab." />
        ) : status.state === "failed" ? (
          <EmptyState icon="alert" title="Could not read the key" body={status.message} />
        ) : (
          <Body
            key={`${key}:${revision}`}
            connectionId={connectionId}
            keyName={key}
            info={status.info}
            readOnly={readOnly}
            onMutated={reloadInfo}
          />
        )}
      </div>
    </div>
  );
}

function Body({ connectionId, keyName, info, readOnly, onMutated }: { connectionId: string; keyName: string; info: RedisKeyInfo; readOnly: boolean; onMutated: () => void }) {
  switch (info.kind) {
    case "string":
    case "json":
      return <DocumentEditor connectionId={connectionId} keyName={keyName} mode={info.kind} readOnly={readOnly} ttlMs={info.ttlMs} onSaved={onMutated} />;
    case "hash":
    case "list":
    case "set":
    case "zset":
      return <CollectionEditor connectionId={connectionId} keyName={keyName} kind={info.kind} readOnly={readOnly} onMutated={onMutated} />;
    case "stream":
      return <StreamView connectionId={connectionId} keyName={keyName} />;
    case "other":
      return <EmptyState icon="key" title={`No editor for ${info.typeName}`} body="This key belongs to a module. Use the query tab to work with it." />;
  }
}

// WHAT:  The memory / encoding / idle / size figures, each only when the server gave it.
function Metadata({ info }: { info: RedisKeyInfo }) {
  const noun = SIZE_NOUN[info.kind];
  const items: { label: string; value: string; hint: string }[] = [];
  if (info.length !== null) items.push({ label: "Size", value: `${formatCount(info.length)}${noun === "" ? "" : ` ${noun}`}`, hint: "Elements in the key (string: length in bytes)" });
  if (info.memoryBytes !== null) items.push({ label: "Memory", value: formatBytes(info.memoryBytes), hint: "MEMORY USAGE" });
  if (info.encoding !== null) items.push({ label: "Encoding", value: info.encoding, hint: "OBJECT ENCODING" });
  if (info.idleSeconds !== null) items.push({ label: "Idle", value: formatIdle(info.idleSeconds), hint: "OBJECT IDLETIME: time since the key was last read or written" });
  if (items.length === 0) return <span className="text-[10px] text-muted/70">Metadata unavailable</span>;
  return (
    <dl className="mr-1 hidden items-center gap-x-2 text-[10px] text-muted md:flex">
      {items.map((item) => (
        <div key={item.label} className="flex items-center gap-1" title={item.hint}>
          <dt>{item.label}</dt>
          <dd className="font-mono text-foreground/80">{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}

// WHAT:  Renames with RENAMENX, so a name that is taken is refused instead of overwritten.
function RenameKey({ connectionId, keyName, disabled, onRenamed }: { connectionId: string; keyName: string; disabled: boolean; onRenamed: (name: string) => void }) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState(keyName);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const apply = async () => {
    if (name === keyName) {
      setOpen(false);
      return;
    }
    if (name.trim() === "") {
      setProblem("Give the key a name.");
      return;
    }
    setBusy(true);
    try {
      const outcome = await runRedis(connectionId, [redisLine("RENAMENX", keyName, name)]);
      if (replyInt(outcome) === 1) {
        showInfo(`Renamed to ${name}.`);
        setOpen(false);
        onRenamed(name);
      } else {
        setProblem(`A key named "${name}" already exists.`);
      }
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setName(keyName);
          setProblem(null);
        }
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label="Rename key" disabled={disabled}>
          <Icon name="pencil" size={14} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 space-y-2 p-2.5">
        <p className="text-xs font-medium text-foreground">Rename key</p>
        <Input
          autoFocus
          aria-label="New key name"
          value={name}
          spellCheck={false}
          onChange={(event) => {
            setName(event.target.value);
            setProblem(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") void apply();
          }}
          className="font-mono text-xs"
        />
        {problem !== null ? <p className="text-xs text-danger">{problem}</p> : null}
        <div className="flex justify-end gap-2">
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setOpen(false);
            }}
          >
            Cancel
          </Button>
          <Button size="sm" pending={busy} onClick={() => void apply()}>
            Rename
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

// WHAT:  A stream's entries in the grid (read-only; streams are appended to, not edited).
function StreamView({ connectionId, keyName }: { connectionId: string; keyName: string }) {
  const density = useWorkspace((s) => s.density);
  const showError = useWorkspace((s) => s.showError);
  const [page, setPage] = useState<TablePage | null>(null);

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const next = await ipc("fetch_table_page", { connectionId, table: keyRef(keyName), query: { sort: [], filters: [], offset: 0, limit: 1000 } });
        if (!token.cancelled) setPage(next);
      } catch (raw) {
        if (!token.cancelled) showError(normalizeError(raw));
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [connectionId, keyName, showError]);

  if (page === null) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner size="sm" />
      </div>
    );
  }
  return <DataGrid columns={page.columns.map((c) => ({ name: c.name, typeName: c.dataType, primaryKey: c.primaryKey }))} rowCount={page.rows.length} getRow={(i) => page.rows[i]} rowHeight={DENSITIES[density].rowHeight} />;
}
