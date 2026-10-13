// SOT: color-swatches, diagram-colour-picker
import type { ConnectionColor } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { DIAGRAM_COLORS, colorLabel, colorVar } from "./palette";

interface ColorSwatchesProps {
  value: ConnectionColor | null;
  onChange: (color: ConnectionColor | null) => void;
  /// Names what "no colour" means here ("Default", "Accent"…); omit to hide the reset swatch.
  resetLabel?: string;
  className?: string;
}

// WHAT:  A row of palette swatches, plus an optional "back to default" one.
// WHY:   Tables, notes and relations all pick from the same nine colours; one
//        control keeps the three inspectors, the bulk bar and the right-click
//        menu identical.
export function ColorSwatches({ value, onChange, resetLabel, className }: ColorSwatchesProps) {
  return (
    <div role="group" aria-label="Colour" className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {resetLabel !== undefined ? (
        <button
          type="button"
          aria-label={resetLabel}
          title={resetLabel}
          aria-pressed={value === null}
          onClick={() => {
            onChange(null);
          }}
          className={cn(
            "flex size-5 items-center justify-center rounded-full border border-border bg-surface-secondary text-muted hover:text-foreground",
            value === null ? "ring-2 ring-ring" : "",
          )}
        >
          <Icon name="x" size={10} />
        </button>
      ) : null}
      {DIAGRAM_COLORS.map((color) => (
        <button
          key={color}
          type="button"
          aria-label={colorLabel(color)}
          title={colorLabel(color)}
          aria-pressed={value === color}
          onClick={() => {
            onChange(color);
          }}
          className={cn("size-5 rounded-full border border-border", value === color ? "ring-2 ring-ring ring-offset-1 ring-offset-background" : "hover:scale-110")}
          style={{ background: colorVar(color) }}
        />
      ))}
    </div>
  );
}
