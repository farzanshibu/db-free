// SOT: datetime-parsing, temporal-kind, db-datetime-text
export type TemporalKind = "date" | "time" | "datetime";

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const TIME_RE = /^\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?$/;
const DATETIME_RE = /^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?$/;
const ZONED_RE = /^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}(?::?\d{2})?)$/i;

// WHAT:  Which temporal control a column gets: from its declared SQL type first,
//        then from the shape of a sample value (engines without typed metadata).
// WHY:   `timestamp` wants a calendar and a clock, `date` a calendar only,
//        `time` a clock only. Every adapter hands date-like values over as DB text.
// WHERE: src/lib/fields.ts, src/components/global/Field.tsx
export function temporalKind(typeName: string, sample?: string): TemporalKind | null {
  const t = typeName.toLowerCase();
  if (/timestamp|datetime/.test(t)) return "datetime";
  if (/\bdate\b|^date/.test(t)) return "date";
  if (/\btime\b|^time/.test(t)) return "time";
  return sample === undefined ? null : kindOfText(sample);
}

export function kindOfText(text: string): TemporalKind | null {
  const s = text.trim();
  if (DATE_RE.test(s)) return "date";
  if (TIME_RE.test(s)) return "time";
  if (DATETIME_RE.test(s) || ZONED_RE.test(s)) return "datetime";
  return null;
}

// WHAT:  A DB temporal literal taken apart into the pieces the pickers edit.
export interface DbTemporal {
  /// The calendar day, as a local Date at midnight — what react-day-picker
  /// selects. `null` when the text holds no date, or is not parseable.
  date: Date | null;
  /// `HH:MM:SS`, or "" when the text holds no time.
  time: string;
  /// The original literal carried an offset or `Z`, so it must go back as an
  /// absolute UTC timestamp rather than a naive one.
  zoned: boolean;
  /// The separator the original used between date and time, preserved so a
  /// round-trip through the editor does not rewrite the column's own style.
  separator: " " | "T";
}

const two = (n: number) => String(n).padStart(2, "0");

// WHAT:  DB text → its parts. Unparseable text yields an empty value rather
//        than throwing: an editor opened on a malformed cell should still open.
export function parseDbTemporal(text: string): DbTemporal {
  const s = text.trim();
  const separator: " " | "T" = s.includes("T") ? "T" : " ";
  const empty: DbTemporal = { date: null, time: "", zoned: false, separator };

  if (TIME_RE.test(s)) return { ...empty, time: normaliseTime(s) };

  if (ZONED_RE.test(s)) {
    // `Date` parses ISO-8601 with an offset; the naive forms below must not go
    // through it, because a bare `YYYY-MM-DD` would be read as UTC and shift a
    // day in western time zones.
    const at = new Date(s.replace(" ", "T"));
    if (Number.isNaN(at.getTime())) return empty;
    return { date: midnight(at), time: `${two(at.getHours())}:${two(at.getMinutes())}:${two(at.getSeconds())}`, zoned: true, separator };
  }

  const parts = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?))?$/.exec(s);
  if (!parts) return empty;
  const [, year, month, day, time] = parts;
  if (year === undefined || month === undefined || day === undefined) return empty;
  return {
    date: new Date(Number(year), Number(month) - 1, Number(day)),
    time: time === undefined ? "" : normaliseTime(time),
    zoned: false,
    separator,
  };
}

// WHAT:  The parts → DB text. An empty value round-trips to "" so clearing a
//        cell stays a NULL rather than becoming the epoch.
export function formatDbTemporal(kind: TemporalKind, value: DbTemporal): string {
  if (kind === "time") return value.time;
  if (value.date === null) return "";
  const date = `${String(value.date.getFullYear()).padStart(4, "0")}-${two(value.date.getMonth() + 1)}-${two(value.date.getDate())}`;
  if (kind === "date") return date;
  const time = value.time === "" ? "00:00:00" : value.time;
  if (value.zoned) {
    const [h = "0", m = "0", sec = "0"] = time.split(":");
    const at = new Date(value.date.getFullYear(), value.date.getMonth(), value.date.getDate(), Number(h), Number(m), Number(sec));
    return at.toISOString();
  }
  return `${date}${value.separator}${time}`;
}

/// `HH:MM` and `HH:MM:SS.fff` both become `HH:MM:SS`: one shape for the editor.
function normaliseTime(text: string): string {
  const [h = "00", m = "00", s = "00"] = text.split(":");
  return `${h}:${m}:${s.split(".")[0] ?? "00"}`;
}

function midnight(at: Date): Date {
  return new Date(at.getFullYear(), at.getMonth(), at.getDate());
}
