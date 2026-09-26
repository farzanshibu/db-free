// SOT: connection-health, session-ping-poll, connection-lost, auto-reconnect
import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, normalizeError } from "@/lib/ipc";
import { cn } from "@/lib/cn";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/// How often the active session is pinged while the window is in use.
const POLL_MS = 30_000;

type Health =
  | { state: "checking" }
  | { state: "ok"; latencyMs: number; at: number }
  | { state: "reconnecting" }
  | { state: "lost"; error: string };

// WHAT:  Latency label, and the dot colour it earns.
function latencyTone(ms: number): "ok" | "slow" {
  return ms < 300 ? "ok" : "slow";
}

// WHAT:  Status line for the active connection: a dot and the round-trip
//        latency, or "Connection lost" with a Reconnect button.
// WHY:   A session dropped by a VPN, a sleeping laptop or a restarted server
//        otherwise surfaces as a cryptic driver error on the next click.
// HOW:   `ping_session` every 30 s, on window focus, and when the connection
//        changes. The first failure reconnects once, silently (same database
//        as the session had); only if that fails too does the user see it.
//        Reconnect then goes through the store's `connect`, which reports why.
// WHERE: src-tauri/src/services/connection.rs (ping), src/stores/workspace.ts (connect)
export function ConnectionHealth() {
  const connection = useActiveConnection();
  const connected = useWorkspace((s) => (connection ? s.sessions.includes(connection.id) : false));
  if (!connection || !connected) return null;
  // Keyed by connection: switching starts a fresh "Checking…" line.
  return <HealthLine key={connection.id} id={connection.id} />;
}

function HealthLine({ id }: { id: string }) {
  const database = useWorkspace((s) => s.sessionInfos[id]?.database ?? null);
  const connect = useWorkspace((s) => s.connect);
  const [health, setHealth] = useState<Health>({ state: "checking" });
  const inFlight = useRef(false);

  const check = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const latencyMs = await ipc("ping_session", { connectionId: id });
      setHealth({ state: "ok", latencyMs, at: Date.now() });
    } catch {
      // One silent retry: most drops are an idle socket the server closed.
      setHealth({ state: "reconnecting" });
      try {
        await ipc("connect", { id, database });
        const latencyMs = await ipc("ping_session", { connectionId: id });
        setHealth({ state: "ok", latencyMs, at: Date.now() });
      } catch (raw) {
        setHealth({ state: "lost", error: normalizeError(raw).message });
      }
    } finally {
      inFlight.current = false;
    }
  }, [id, database]);

  useEffect(() => {
    // First check on the next tick (a subscription callback, like the poll).
    const first = window.setTimeout(() => void check(), 0);
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void check();
    }, POLL_MS);
    const onFocus = () => void check();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    };
  }, [check]);

  const reconnect = async () => {
    setHealth({ state: "reconnecting" });
    const ok = await connect(id, database ?? undefined);
    if (ok) await check();
    else setHealth({ state: "lost", error: "Reconnect failed." });
  };

  if (health.state === "lost") {
    return (
      <div role="status" className="flex items-center gap-2 border-t border-border/40 px-3 py-2 text-xs">
        <span aria-hidden className="size-2 shrink-0 rounded-full bg-danger" />
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="min-w-0 flex-1 truncate font-medium text-danger">Connection lost</span>
          </TooltipTrigger>
          <TooltipContent className="max-w-80 font-mono text-[11px]">{health.error}</TooltipContent>
        </Tooltip>
        <Button size="sm" variant="secondary" onClick={() => void reconnect()} className="h-6 rounded-md px-2 text-[11px]">
          <Icon name="refresh" size={11} />
          Reconnect
        </Button>
      </div>
    );
  }

  const label =
    health.state === "ok" ? `${health.latencyMs} ms` : health.state === "reconnecting" ? "Reconnecting…" : "Checking…";
  return (
    <div role="status" className="flex items-center gap-2 border-t border-border/40 px-3 py-2 text-[11px] text-muted">
      {health.state === "ok" ? (
        <span aria-hidden className={cn("size-2 shrink-0 rounded-full", latencyTone(health.latencyMs) === "ok" ? "bg-success" : "bg-warning")} />
      ) : (
        <Spinner size="sm" />
      )}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="sm" onClick={() => void check()} className="h-5 min-w-0 shrink gap-1 px-1 text-[11px] font-normal text-muted hover:text-foreground">
            <span className="truncate">{health.state === "ok" ? "Connected" : label}</span>
            {health.state === "ok" ? <span className="font-mono tabular-nums">· {label}</span> : null}
          </Button>
        </TooltipTrigger>
        <TooltipContent>Round-trip to the server. Checked every 30 s and when the window gets focus; click to check now.</TooltipContent>
      </Tooltip>
    </div>
  );
}
