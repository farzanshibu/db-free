// SOT: badge-component, env-badge, pill
import { Badge } from "@/components/ui/badge";
import type { Environment } from "@/lib/bindings";
import { environmentMeta } from "@/lib/environments";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

export function EnvBadge({ environment, readOnly = false }: { environment: Environment; readOnly?: boolean }) {
  const meta = environmentMeta(environment);
  if (environment === "none" && !readOnly) return null;
  return (
    <Badge variant="soft" className={cn("gap-1.5 rounded-full border border-border/60 backdrop-blur-sm", meta.text)}>
      {environment !== "none" ? <span className={cn("size-1.5 rounded-full shadow-xs", meta.dot)} /> : null}
      {environment !== "none" ? meta.label : null}
      {readOnly ? <Icon name="lock" size={11} /> : null}
    </Badge>
  );
}

export function EnvDot({ environment, live }: { environment: Environment; live: boolean }) {
  const meta = environmentMeta(environment);
  return (
    <span
      className={cn("inline-block size-2 shrink-0 rounded-full transition-opacity duration-200", meta.dot, live ? "shadow-xs opacity-100" : "opacity-35")}
      title={`${meta.label}${live ? " · connected" : ""}`}
    />
  );
}
