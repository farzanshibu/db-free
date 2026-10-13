// SOT: grid-find, find-in-page-rows, find-match-stepping

export interface FindMatch {
  row: number;
  col: number;
}

export interface FindResult {
  matches: FindMatch[];
  /// More cells matched than were collected.
  capped: boolean;
}

/// Most matches kept; highlighting past this is noise and the key set gets large.
export const FIND_CAP = 5000;

export const matchKey = (match: FindMatch): string => `${match.row}:${match.col}`;

// WHAT:  Every cell of the loaded page whose text contains the query, in reading order.
// WHY:   ⌘F in a grid means "this page, what I can see", with a count and a way to
//        step through the hits; the database is not searched.
// HOW:   `textAt` returns the cell as the grid shows it, or null for a cell with no
//        text to match (NULL). An empty query matches nothing.
export function findInGrid(
  rowCount: number,
  colCount: number,
  textAt: (row: number, col: number) => string | null,
  query: string,
  matchCase: boolean,
  cap: number = FIND_CAP,
): FindResult {
  if (query.length === 0) return { matches: [], capped: false };
  const needle = matchCase ? query : query.toLowerCase();
  const matches: FindMatch[] = [];
  for (let row = 0; row < rowCount; row += 1) {
    for (let col = 0; col < colCount; col += 1) {
      const text = textAt(row, col);
      if (text === null) continue;
      if ((matchCase ? text : text.toLowerCase()).includes(needle)) {
        if (matches.length >= cap) return { matches, capped: true };
        matches.push({ row, col });
      }
    }
  }
  return { matches, capped: false };
}

/// The next (+1) or previous (-1) hit, wrapping at both ends.
export function stepMatch(current: number, total: number, delta: 1 | -1): number {
  if (total === 0) return 0;
  return (current + delta + total) % total;
}
