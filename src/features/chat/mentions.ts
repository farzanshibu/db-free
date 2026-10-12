// SOT: chat-mentions, at-mention-parsing, mention-candidates, mention-matching
import type { ChatMention, MentionKind } from "@/lib/bindings";

// WHAT:  The pure half of `@` mentions: finding the mention being typed,
//        ranking what it could mean, writing it into the text, and reading
//        mentions back out of text.
// WHY:   The popover, the send button and the seed from other features all need
//        the same answers; keeping them out of the component makes them
//        checkable on their own.
// HOW:   A mention's text is `@` plus its label with spaces turned into
//        underscores, so a token never contains whitespace and "still in the
//        message" is a plain substring test. The server resolves the mention
//        by id; the token is only what the reader sees.
// WHERE: src/features/chat/MentionInput.tsx, src/features/chat/ChatTab.tsx

export interface MentionCandidate {
  kind: MentionKind;
  /// What the server resolves: a table's qualified name, otherwise a record id.
  id: string;
  /// The token text, without the `@`.
  label: string;
  /// Small print in the list: the saved query's first line, a kind name.
  detail: string | null;
}

/// The `@word` being typed at the caret: where it starts and ends, and the word.
export interface ActiveMention {
  start: number;
  end: number;
  query: string;
}

export const MENTION_KIND_LABEL: Record<MentionKind, string> = {
  table: "Tables",
  saved_query: "Saved queries",
  dashboard: "Dashboards",
  workflow: "Workflows",
  diagram: "Diagrams",
};

const KIND_ORDER: readonly MentionKind[] = ["table", "saved_query", "dashboard", "workflow", "diagram"];

/// A label as it appears after the `@`.
export function tokenOf(label: string): string {
  return label.trim().replace(/\s+/g, "_");
}

/// The `@word` ending at the caret, when the caret is inside one.
export function activeMention(text: string, caret: number): ActiveMention | null {
  const before = text.slice(0, caret);
  const match = /(?:^|[\s(,])@([^\s@]*)$/.exec(before);
  if (match === null) return null;
  const query = match[1] ?? "";
  return { start: caret - query.length - 1, end: caret, query };
}

/// How well `query` matches `label`: 0 is no match. Prefix beats substring beats
/// scattered letters, and shorter labels beat longer ones within a tier.
export function fuzzyScore(label: string, query: string): number {
  if (query === "") return 1;
  const l = label.toLowerCase();
  const q = query.toLowerCase();
  if (l === q) return 100;
  if (l.startsWith(q)) return 80 - Math.min(l.length - q.length, 30) / 2;
  const at = l.indexOf(q);
  if (at >= 0) return 60 - Math.min(at, 30) / 2;
  let matched = 0;
  for (const ch of l) {
    if (matched < q.length && ch === q[matched]) matched += 1;
  }
  return matched === q.length ? 20 : 0;
}

/// What the popover lists. No query: a few of each kind. A query: the best matches overall.
export function filterCandidates(
  candidates: readonly MentionCandidate[],
  query: string,
  limit = 10,
): MentionCandidate[] {
  if (query === "") {
    const perKind = 5;
    const out: MentionCandidate[] = [];
    for (const kind of KIND_ORDER) {
      out.push(...candidates.filter((c) => c.kind === kind).slice(0, perKind));
    }
    return out.slice(0, 15);
  }
  return candidates
    .map((candidate) => ({ candidate, score: fuzzyScore(candidate.label, query) }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((entry) => entry.candidate);
}

/// Writes the chosen candidate over the `@word` being typed.
export function applyMention(
  text: string,
  active: ActiveMention,
  candidate: MentionCandidate,
): { text: string; caret: number } {
  const token = `@${candidate.label} `;
  const next = text.slice(0, active.start) + token + text.slice(active.end);
  return { text: next, caret: active.start + token.length };
}

export function mentionOf(candidate: MentionCandidate): ChatMention {
  return { kind: candidate.kind, id: candidate.id, label: candidate.label };
}

/// Whether the token at `at` is a whole word: not glued to the text around it.
function isWholeToken(text: string, at: number, length: number): boolean {
  const before = at === 0 ? " " : text.charAt(at - 1);
  const after = text.charAt(at + length);
  return /[\s(,]/.test(before) && (after === "" || !/[\w.\-]/.test(after));
}

/// Where each whole-word occurrence of `@label` starts.
function tokenStarts(text: string, label: string): number[] {
  const token = `@${label}`;
  const starts: number[] = [];
  let from = 0;
  for (;;) {
    const at = text.indexOf(token, from);
    if (at < 0) return starts;
    if (isWholeToken(text, at, token.length)) starts.push(at);
    from = at + 1;
  }
}

/// True when `@label` appears in `text` as a whole word.
export function referencesMention(text: string, label: string): boolean {
  return tokenStarts(text, label).length > 0;
}

/// The mentions still present in the text (a deleted token drops its mention).
export function mentionsInText(text: string, mentions: readonly ChatMention[]): ChatMention[] {
  return mentions.filter((m) => referencesMention(text, m.label));
}

/// Text written elsewhere (a prompt another feature handed to the chat) with the
/// mentions it names: each `@word` that matches a known candidate becomes one. A
/// table may be written bare (`@orders`) when no other table shares the name; the
/// returned text spells it out (`@public.orders`) so it matches its mention.
export function mentionsFromText(
  text: string,
  candidates: readonly MentionCandidate[],
): { text: string; mentions: ChatMention[] } {
  const mentions: ChatMention[] = [];
  let out = text;
  const bare = (candidate: MentionCandidate): string => candidate.label.slice(candidate.label.lastIndexOf(".") + 1);
  for (const candidate of candidates) {
    if (mentions.some((m) => m.kind === candidate.kind && m.id === candidate.id)) continue;
    if (referencesMention(out, candidate.label)) {
      mentions.push(mentionOf(candidate));
      continue;
    }
    if (candidate.kind !== "table") continue;
    const name = bare(candidate);
    const unique = candidates.filter((other) => other.kind === "table" && bare(other) === name).length === 1;
    if (!unique || name === candidate.label || !referencesMention(out, name)) continue;
    for (const start of tokenStarts(out, name).reverse()) {
      out = `${out.slice(0, start)}@${candidate.label}${out.slice(start + name.length + 1)}`;
    }
    mentions.push(mentionOf(candidate));
  }
  return { text: out, mentions };
}

export interface TextPart {
  text: string;
  mention: ChatMention | null;
}

/// The message split into plain runs and mention tokens, so the tokens can be styled.
export function splitMentions(text: string, mentions: readonly ChatMention[]): TextPart[] {
  const spans: { start: number; end: number; mention: ChatMention }[] = [];
  for (const mention of mentions) {
    for (const start of tokenStarts(text, mention.label)) {
      const end = start + mention.label.length + 1;
      if (!spans.some((s) => start < s.end && end > s.start)) spans.push({ start, end, mention });
    }
  }
  spans.sort((a, b) => a.start - b.start);
  const parts: TextPart[] = [];
  let cursor = 0;
  for (const span of spans) {
    if (span.start > cursor) parts.push({ text: text.slice(cursor, span.start), mention: null });
    parts.push({ text: text.slice(span.start, span.end), mention: span.mention });
    cursor = span.end;
  }
  if (cursor < text.length) parts.push({ text: text.slice(cursor), mention: null });
  return parts.length > 0 ? parts : [{ text, mention: null }];
}
