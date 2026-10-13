// SOT: conditions-editor, condition-rule-row, condition-live-highlight
import type { QueryOutcome, Widget, WidgetCondition } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { AppSelect, Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { CONDITION_OPS, matchedIndex, newCondition, opNeedsValue } from "./conditions";
import { TileColorPicker } from "./TileColorPicker";

// WHAT:  The rule list of a metric, text, image or GIF tile: "when the first
//        cell <operator> <value>, show <content> on <background>".
// WHY:   Rules decide what the tile shows from its own data. Marking the rule
//        that matches the *current* result makes "last match wins" visible while
//        editing instead of something to reason out.
// HOW:   The matched rule (the last whose test passes against the result the
//        inspector last ran) is outlined and badged. `is null` / `is not null`
//        take no value. A metric rule carries a background only; text carries
//        content and a background; image and GIF carry a URL.
// WHERE: ./conditions.ts (matching), ./TileInspector.tsx

export function ConditionsEditor({ widget, outcome, onChange }: { widget: Widget; outcome: QueryOutcome | null; onChange: (next: WidgetCondition[]) => void }) {
  const matched = matchedIndex(widget.conditions, outcome);
  const patch = (i: number, partial: Partial<WidgetCondition>) => { onChange(widget.conditions.map((c, j) => (j === i ? { ...c, ...partial } : c))); };
  const isMetric = widget.kind === "metric";
  const isText = widget.kind === "text";
  const contentLabel = isText ? "Content to display" : "URL to display";
  return (
    <div className="flex flex-col gap-2 border-b border-border px-2.5 py-1.5 text-xs">
      <span className="font-medium text-foreground">Conditions</span>
      <p className="text-muted">Evaluates the first column of the first row. Last match wins.</p>
      {widget.conditions.map((c, i) => {
        const isMatch = i === matched;
        return (
          <div key={i} className={cn("flex flex-col gap-2 rounded-lg border p-2 transition-colors", isMatch ? "border-success bg-success-soft" : "border-border bg-background")}>
            <div className="flex items-center gap-2">
              <Icon name="git-branch" size={12} className={isMatch ? "text-success" : "text-muted"} />
              <span className={cn("text-[10px] font-medium tracking-wide uppercase", isMatch ? "text-success" : "text-muted")}>
                Condition {i + 1}
                {isMatch ? " - matched" : ""}
              </span>
              <span className="ml-auto">
                <Button variant="ghost" size="icon-sm" aria-label="Remove condition" onClick={() => { onChange(widget.conditions.filter((_, j) => j !== i)); }}>
                  <Icon name="x" />
                </Button>
              </span>
            </div>
            <div className="flex items-center gap-2">
              <AppSelect ariaLabel="Operator" value={c.op} options={CONDITION_OPS} onChange={(op) => { patch(i, { op }); }} size="sm" className="w-40 shrink-0" />
              {opNeedsValue(c.op) ? <Field label="" value={c.value} onChange={(value) => { patch(i, { value }); }} placeholder="value" className="[&_label]:hidden" mono /> : null}
            </div>
            {isMetric ? null : (
              <Field
                label=""
                value={c.content}
                onChange={(content) => { patch(i, { content }); }}
                placeholder={contentLabel}
                description="Use {{column_name}} or {{a / b * 100}} to insert query values"
                className="[&_label]:hidden"
                mono={!isText}
              />
            )}
            {isMetric || isText ? (
              <div className="flex items-center gap-2">
                <span className="text-muted">Background</span>
                <TileColorPicker kind="background" label={`Condition ${i + 1} background`} value={c.bg} onChange={(bg) => { patch(i, isMetric ? { bg, content: "" } : { bg }); }} />
              </div>
            ) : null}
          </div>
        );
      })}
      <Button size="sm" variant="secondary" className="self-start" onClick={() => { onChange([...widget.conditions, newCondition("")]); }}>
        <Icon name="plus" size={12} />
        Add Condition
      </Button>
    </div>
  );
}
