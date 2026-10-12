// SOT: row-count-label, compact-count, pager-total
import { formatCount } from "@/lib/format";

// WHAT:  Row counts the way the pager shows them: 842, 1.2K, 3.4M.
// WHY:   "3,412,907 rows" is noise next to a page of 50; the magnitude is what
//        the eye wants, and the tooltip carries the exact figure.
export function compactCount(n: number): string {
  const abs = Math.abs(n);
  if (abs < 1000) return String(n);
  const units: readonly (readonly [number, string])[] = [
    [1e3, "K"],
    [1e6, "M"],
    [1e9, "B"],
    [1e12, "T"],
  ];
  let index = 0;
  units.forEach(([size], i) => {
    if (abs >= size) index = i;
  });
  const format = (i: number): { text: string; rounded: number } => {
    const [size, suffix] = units[i] ?? units[0] ?? [1e3, "K"];
    const scaled = n / size;
    // 1.2K but 12K and 120K: one decimal only while it is small enough to matter.
    const rounded = Math.abs(scaled) < 10 ? Math.round(scaled * 10) / 10 : Math.round(scaled);
    return { text: `${rounded}${suffix}`, rounded: Math.abs(rounded) };
  };
  const first = format(index);
  // 999,999 rounds to 1000K: say 1M instead.
  if (first.rounded >= 1000 && index + 1 < units.length) return format(index + 1).text;
  return first.text;
}

export interface RowTotal {
  /// What the pager prints, e.g. "3.4M+" for an estimate or "842" for an exact count.
  label: string;
  /// The longer sentence for the tooltip.
  title: string;
}

// WHAT:  The pager's row-count text. An estimate carries a "+" and says so in its tooltip.
export function rowTotal(total: number | null, exact: boolean, loaded: number): RowTotal {
  if (total === null) return { label: compactCount(loaded), title: `${formatCount(loaded)} rows on this page; the total is not known` };
  if (exact) return { label: compactCount(total), title: `${formatCount(total)} rows` };
  return { label: `${compactCount(total)}+`, title: `About ${formatCount(total)} rows (estimated from the table statistics)` };
}
