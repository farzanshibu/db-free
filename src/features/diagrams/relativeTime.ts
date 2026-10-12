// SOT: relative-time, time-ago-label
// WHAT:  "Just now", "5m ago", "3h ago", "Yesterday", "4d ago", then a calendar date.
// WHY:   The version history lists when each state was saved; an absolute timestamp
//        makes the reader do the subtraction.
// HOW:   Pure on (ISO timestamp, now) so the label is the same wherever it is drawn.
export function relativeTime(iso: string, now: number = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.floor((now - then) / 1000));
  if (seconds < 60) return "Just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days}d ago`;
  const options: Intl.DateTimeFormatOptions = days > 300 ? { month: "short", day: "numeric", year: "numeric" } : { month: "short", day: "numeric" };
  return new Date(then).toLocaleDateString(undefined, options);
}
