// SOT: relative-time, time-ago-label

// WHAT:  "Just now", "5m ago", "3h ago", "Yesterday", "4d ago", then a short date.
// WHY:   Version lists and Home recents care about how long ago, not the timestamp.
// HOW:   `now` is a parameter so the function stays pure (and testable).
export function relativeTime(iso: string, now: number = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) return "Just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days}d ago`;
  const date = new Date(then);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return sameYear
    ? date.toLocaleDateString(undefined, { month: "short", day: "numeric" })
    : date.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}
