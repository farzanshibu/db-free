// SOT: tile-appearance, tile-background, matched-condition-style, change-colouring
import type { QueryOutcome, Widget, WidgetCondition, WidgetKind } from "@/lib/bindings";
import { matchedCondition } from "./conditions";
import { metricStats, type MetricStats } from "./metric";
import { gradientCss, isSwatch, tintCss, CHANGE_DOWN, CHANGE_UP } from "./tiles";

// WHAT:  How a tile is dressed for its current result: the accent colour, the
//        background behind it, and whether text on it is white.
// WHY:   The frame (its header and edge), the body and the inspector's preview
//        must agree, and the answer depends on three things at once: the tile's
//        own colour, the condition rule that matches now, and the sign of a
//        metric's change. Working it out once, here, keeps them the same.
// HOW:   Precedence for the background: a metric coloured by its change, then
//        the matched rule's swatch, then the tile's. The accent follows the
//        matched rule's legacy tint (older metric rules stored one in `content`)
//        and otherwise the tile's tint.
// WHERE: ./DashboardGrid.tsx (frame), ./charts.tsx (bodies), ./TileInspector.tsx

export interface Appearance {
  /// The rule that matches the result now, if any.
  condition: WidgetCondition | null;
  /// Solid colour for values, lines, bars and the tile's top edge.
  accent: string;
  /// CSS `background` for the whole tile, or undefined for the plain surface.
  background: string | undefined;
  /// The background is a swatch gradient, so text on it is white.
  onGradient: boolean;
  /// A metric's numbers, worked out once for the frame and the body.
  metric: MetricStats | null;
}

const BACKGROUND_KINDS: readonly WidgetKind[] = ["metric", "text", "image", "gif"];
const CONDITION_KINDS: readonly WidgetKind[] = ["metric", "text", "image", "gif"];

/// A faint wash of a swatch, for "colour by change": the tile stays readable on
/// the surface it sits on rather than turning into a solid block.
function changeWash(name: string): string {
  return `linear-gradient(135deg, color-mix(in oklch, var(--tile-${name}) 24%, transparent), color-mix(in oklch, var(--tile-${name}) 10%, transparent))`;
}

export function tileAppearance(widget: Widget, outcome: QueryOutcome | null): Appearance {
  const condition = CONDITION_KINDS.includes(widget.kind) ? matchedCondition(widget.conditions, outcome) : null;
  const legacyTint = widget.kind === "metric" && condition !== null && (isSwatch(condition.content) || condition.content.startsWith("series-")) ? condition.content : null;
  const accent = tintCss(legacyTint ?? widget.tint);
  const metric = widget.kind === "metric" ? metricStats(outcome) : null;
  const wantsBackground = BACKGROUND_KINDS.includes(widget.kind);
  if (wantsBackground && widget.kind === "metric" && widget.colorByChange && widget.showChange && metric?.change !== null && metric?.change !== undefined) {
    return { condition, accent, background: changeWash(metric.change >= 0 ? CHANGE_UP : CHANGE_DOWN), onGradient: false, metric };
  }
  const gradient = wantsBackground ? gradientCss(condition?.bg ?? widget.bg) : undefined;
  return { condition, accent, background: gradient, onGradient: gradient !== undefined, metric };
}
