// SOT: fuzzy-match, fuzzy-score, palette-ranking, sql-detection, recent-ids

// WHAT:  Typo-tolerant matching for the command palette: a score for how well a
//        query fits a label, and the test for "this looks like SQL, not a command".
// WHY:   `includes()` made "nqt" find nothing and ranked "Settings" and "Query
//        Settings" alike for "set". Subsequence matching with bonuses for the
//        start of the label, word starts and unbroken runs ranks the way people
//        expect: the exact word first, the scattered letters last.
// HOW:   Case-insensitive. Every whitespace-separated word of the query must
//        match (so "new query" finds "New query tab"); a word matches as a plain
//        substring, which scores highest, or as a subsequence. Pure and quick
//        enough to run on every keystroke across a few thousand tables.
// WHERE: src/features/palette/CommandPalette.tsx

const WORD_START = /[\s\-_.:/]/;

// Bonuses, in points. They only need to be ordered sensibly against each other.
const BONUS_PREFIX = 120;
const BONUS_SUBSTRING = 70;
const BONUS_WORD_START = 12;
const BONUS_RUN = 8;
const PENALTY_GAP = 1;
const PENALTY_LENGTH = 0.25;

/// Score of one word against a text, or null when its letters are not all there in order.
function scoreWord(word: string, text: string): number | null {
  if (word.length === 0) return 0;
  const at = text.indexOf(word);
  if (at === 0) return BONUS_PREFIX + BONUS_RUN * word.length - PENALTY_LENGTH * text.length;
  if (at > 0) {
    const boundary = WORD_START.test(text.charAt(at - 1)) ? BONUS_WORD_START : 0;
    return BONUS_SUBSTRING + boundary + BONUS_RUN * word.length - PENALTY_LENGTH * text.length - at * PENALTY_GAP * 0.5;
  }
  // Subsequence: walk the text once, taking each letter at its first chance.
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const letter of word) {
    const found = text.indexOf(letter, from);
    if (found < 0) return null;
    score += 1;
    if (found === previous + 1) score += BONUS_RUN;
    if (found === 0 || WORD_START.test(text.charAt(found - 1))) score += BONUS_WORD_START;
    score -= Math.min(found - from, 8) * PENALTY_GAP;
    previous = found;
    from = found + 1;
  }
  return score - PENALTY_LENGTH * text.length;
}

/// How well `query` fits `text` (higher is better), or null when it does not fit at all.
export function fuzzyScore(query: string, text: string): number | null {
  const words = query.toLowerCase().split(/\s+/).filter((w) => w.length > 0);
  if (words.length === 0) return 0;
  const haystack = text.toLowerCase();
  let total = 0;
  for (const word of words) {
    const score = scoreWord(word, haystack);
    if (score === null) return null;
    total += score;
  }
  return total;
}

/// The best of the label and the (weaker) hint, or null when neither fits.
export function bestScore(query: string, label: string, hint: string | undefined): number | null {
  const onLabel = fuzzyScore(query, label);
  const onHint = hint === undefined ? null : fuzzyScore(query, hint);
  if (onLabel === null && onHint === null) return null;
  return Math.max(onLabel ?? Number.NEGATIVE_INFINITY, onHint === null ? Number.NEGATIVE_INFINITY : onHint * 0.4);
}

const SQL_START =
  /^\s*(?:select|with|insert|update|delete|replace|merge|create|alter|drop|truncate|explain|show|describe|desc|pragma|call|exec|execute|grant|revoke|vacuum|analyze|use|set|begin|commit|rollback)\s+\S/i;

/// Whether palette input reads as a statement to run rather than something to search for.
export function looksLikeSql(input: string): boolean {
  return SQL_START.test(input);
}
