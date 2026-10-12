// SOT: redis-key-formatting, ttl-units, byte-size-format, idle-time-format
import type { RedisKeyType } from "@/lib/bindings";
import type { IconName } from "@/lib/icons";

// WHAT:  How the key tab and the key tree word what Redis reports.
// WHY:   Memory, idle time and expiry are numbers the user reads at a glance;
//        one set of formatters keeps "1.5 KB" and "1h 5m" the same everywhere.
// WHERE: src/features/keys/KeyTab.tsx, src/features/shell/KeyTree.tsx

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Number((bytes / 1024).toFixed(1))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${Number((bytes / (1024 * 1024)).toFixed(1))} MB`;
  return `${Number((bytes / (1024 * 1024 * 1024)).toFixed(1))} GB`;
}

// WHAT:  Seconds since the key was last read or written: `45s`, `3m`, `2h 10m`, `4d 1h`.
export function formatIdle(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  return `${minutes}m`;
}

// WHAT:  Time left on a key, down to the second: `3d 4h`, `59m 3s`, `12s`, `340 ms`.
export function formatRemaining(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  const total = Math.floor(ms / 1000);
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  const seconds = total % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

export type TtlUnit = "s" | "m" | "h" | "d";

export const TTL_UNITS: readonly { value: TtlUnit; label: string; seconds: number }[] = [
  { value: "s", label: "seconds", seconds: 1 },
  { value: "m", label: "minutes", seconds: 60 },
  { value: "h", label: "hours", seconds: 3_600 },
  { value: "d", label: "days", seconds: 86_400 },
];

// WHAT:  A number and a unit → whole seconds, or null when it is not a positive whole number.
export function ttlSeconds(amount: number | null, unit: TtlUnit): number | null {
  if (amount === null || !Number.isFinite(amount) || amount <= 0) return null;
  const seconds = amount * (TTL_UNITS.find((u) => u.value === unit)?.seconds ?? 1);
  return Number.isSafeInteger(seconds) ? seconds : null;
}

export const KEY_TYPE_LABEL: Record<RedisKeyType, string> = {
  string: "String",
  hash: "Hash",
  list: "List",
  set: "Set",
  zset: "Sorted set",
  stream: "Stream",
  json: "JSON",
  other: "Other",
};

// WHAT:  The icon the tree and the tab header draw for a key type.
export const KEY_TYPE_ICON: Record<RedisKeyType, IconName> = {
  string: "text",
  hash: "grid",
  list: "list",
  set: "layers",
  zset: "sort",
  stream: "rss",
  json: "braces",
  other: "key",
};
