// SOT: badge-component, env-badge, pill
import { Badge } from "@/components/ui/badge";
import type { Environment } from "@/lib/bindings";
import { environmentMeta } from "@/lib/environments";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

export function EnvBadge({ environment, readOnly = false }: { environment: Environment; readOnly?: boolean }) {
  const meta = environmentMeta(environment);
  if (environment === "none" && !readOnly) return null;
  // WHAT:  Read-only with no environment to name is just the lock.
  // WHY:   A bordered pill wrapped around a single 11px glyph reads as an empty
  //        box, especially next to an engine's brand tile in a narrow sidebar.
  if (environment === "none") {
    return <Icon name="lock" size={12} className="shrink-0 text-muted" aria-label="Read-only connection" />;
  }
  // Past the guard above, the environment always has a name to show.
  return (
    <Badge variant="soft" className={cn("gap-1.5 rounded-full border border-border/60 backdrop-blur-sm", meta.text)}>
      <span className={cn("size-1.5 rounded-full", meta.dot)} />
      {meta.label}
      {readOnly ? <Icon name="lock" size={11} /> : null}
    </Badge>
  );
}

export function EnvDot({ environment, live }: { environment: Environment; live: boolean }) {
  const meta = environmentMeta(environment);
  // WHAT:  Live-connection dot: the environment colour at full opacity, or green
  //        when the connection has no environment to name — otherwise a live
  //        "none" connection stays grey and the connected state is invisible.
  // WHY:   The query tab and the switcher both render this; a stale session dims
  //        to 35% so connected vs disconnected never depends on hue alone.
  const dot = live && environment === "none" ? "bg-success" : meta.dot;
  return (
    <span
      className={cn("inline-block size-2 shrink-0 rounded-full transition-opacity duration-200", dot, live ? "shadow-xs opacity-100" : "opacity-35")}
      title={`${meta.label}${live ? " · connected" : ""}`}
    />
  );
}
