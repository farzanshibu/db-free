// SOT: document-editor, redis-string-editor, redisjson-editor, dirty-tracking, keep-ttl-save
import { useCallback, useEffect, useMemo, useState } from "react";
import type { Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { parseJson } from "@/lib/json";
import { redisLine } from "@/lib/redisCommands";
import { useWorkspace } from "@/stores/workspace";
import { EmptyState } from "@/components/global/EmptyState";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { keyRef, runRedis } from "./keyCommands";
import { ValueCodeEditor } from "./ValueCodeEditor";

// WHAT:  The editor for a key that holds one document: a Redis string or a
//        RedisJSON value. Load, edit with highlighting, see when it differs from
//        the server's copy, Save or Revert.
// WHY:   Strings are the commonest key and the one users edit most; losing the
//        key's expiry on every save (a plain SET clears it) would be a quiet bug.
// HOW:   A string saves with `SET key value KEEPTTL` (Redis 6+); on a server that
//        rejects KEEPTTL it falls back to SET and puts the remaining TTL back. A
//        JSON key saves with `JSON.SET key $ <compact JSON>` after the text
//        parses. JSON that parses can be formatted or minified in place.
// WHERE: src-tauri/src/integrations/redis.rs (fetch_page loads the value)
interface DocumentEditorProps {
  connectionId: string;
  keyName: string;
  mode: "string" | "json";
  readOnly: boolean;
  /// Remaining time to live in milliseconds (-1: none), to restore after a SET on an old server.
  ttlMs: number;
  onSaved: () => void;
}

type Loaded =
  | { state: "loading" }
  | { state: "failed"; message: string }
  | { state: "binary"; base64: string; bytes: number }
  | { state: "ready"; original: string };

function documentText(cell: Value | undefined, mode: "string" | "json"): { text: string } | { base64: string } {
  if (cell === undefined || cell.t === "null") return { text: "" };
  if (cell.t === "bytes") return { base64: cell.v };
  if (cell.t === "json") return { text: JSON.stringify(cell.v, null, mode === "json" || typeof cell.v === "object" ? 2 : undefined) };
  return { text: String(cell.v) };
}

function byteLength(base64: string): number {
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((base64.length * 3) / 4) - padding);
}

export function DocumentEditor({ connectionId, keyName, mode, readOnly, ttlMs, onSaved }: DocumentEditorProps) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [loaded, setLoaded] = useState<Loaded>({ state: "loading" });
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    const token = { stale: false };
    void (async () => {
      try {
        const page = await ipc("fetch_table_page", { connectionId, table: keyRef(keyName), query: { sort: [], filters: [], offset: 0, limit: 1 } });
        if (token.stale) return;
        const found = documentText(page.rows[0]?.[0], mode);
        if ("base64" in found) {
          setLoaded({ state: "binary", base64: found.base64, bytes: byteLength(found.base64) });
          setText(found.base64);
        } else {
          setLoaded({ state: "ready", original: found.text });
          setText(found.text);
        }
        setProblem(null);
      } catch (raw) {
        if (!token.stale) setLoaded({ state: "failed", message: normalizeError(raw).message });
      }
    })();
    return () => {
      token.stale = true;
    };
  }, [connectionId, keyName, mode, attempt]);

  const original = loaded.state === "ready" ? loaded.original : null;
  const dirty = original !== null && text !== original;
  // WHAT:  The text as a JSON object or array, when it is one; drives Format / Minify
  //        and the highlighting of a string key that holds a document.
  const parsed = useMemo(() => {
    const value = parseJson(text);
    return typeof value === "object" && value !== null ? { value } : null;
  }, [text]);

  const save = useCallback(async () => {
    if (!dirty || readOnly) return;
    setSaving(true);
    setProblem(null);
    try {
      if (mode === "json") {
        const value = parseJson(text);
        if (value === undefined) {
          setProblem("This is not valid JSON, so it cannot be saved.");
          return;
        }
        await runRedis(connectionId, [redisLine("JSON.SET", keyName, "$", JSON.stringify(value))]);
      } else {
        try {
          await runRedis(connectionId, [redisLine("SET", keyName, text, "KEEPTTL")]);
        } catch (raw) {
          if (!normalizeError(raw).message.toLowerCase().includes("syntax error")) throw raw;
          const lines = [redisLine("SET", keyName, text)];
          if (ttlMs > 0) lines.push(redisLine("PEXPIRE", keyName, String(ttlMs)));
          await runRedis(connectionId, lines);
        }
      }
      setLoaded({ state: "ready", original: text });
      showInfo(`Saved ${keyName}.`);
      onSaved();
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSaving(false);
    }
  }, [dirty, readOnly, mode, text, connectionId, keyName, ttlMs, showInfo, showError, onSaved]);

  if (loaded.state === "loading") {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner size="sm" />
      </div>
    );
  }
  if (loaded.state === "failed") {
    return <EmptyState icon="alert" title="Could not load the value" body={loaded.message} action={
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              setLoaded({ state: "loading" });
              setAttempt((n) => n + 1);
            }}
          >
            Try again
          </Button>
        } />;
  }

  const binary = loaded.state === "binary";
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border px-2.5 text-xs">
        <Button size="sm" onClick={() => void save()} disabled={readOnly || !dirty} pending={saving}>
          <Icon name="check" size={12} />
          Save
        </Button>
        <Button size="sm" variant="secondary" disabled={!dirty || saving} onClick={() => original !== null && setText(original)}>
          Revert
        </Button>
        {parsed !== null && !binary ? (
          <>
            <Button size="sm" variant="ghost" disabled={readOnly} onClick={() => setText(JSON.stringify(parsed.value, null, 2))}>
              Format
            </Button>
            <Button size="sm" variant="ghost" disabled={readOnly} onClick={() => setText(JSON.stringify(parsed.value))}>
              Minify
            </Button>
          </>
        ) : null}
        {dirty ? <span className="text-warning">Unsaved changes</span> : null}
        {problem !== null ? <span className="truncate text-danger">{problem}</span> : null}
        <span className="ml-auto text-muted">
          {binary ? `Binary value, ${loaded.bytes.toLocaleString()} bytes (shown as base64, read-only)` : `${text.length.toLocaleString()} characters`}
          {readOnly ? " · read-only connection" : ""}
        </span>
      </div>
      <div className="min-h-0 flex-1">
        <ValueCodeEditor key={`${keyName}:${mode}`} value={text} onChange={setText} language={mode === "json" || parsed !== null ? "json" : "text"} readOnly={readOnly || binary} ariaLabel="Value" onSave={() => void save()} />
      </div>
    </div>
  );
}
