// SOT: tile-colour-picker, swatch-popover
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { TILE_SWATCHES, gradientCss, isSwatch, swatchLabel, tintCss } from "./tiles";

// WHAT:  A round swatch button that opens the 14 tile colours plus "none".
// WHY:   Tints, tile backgrounds and condition backgrounds all pick from the same
//        registry; one picker keeps them identical and keeps the choice to a
//        name (`red`), which each theme then draws in its own tuned shade.
// HOW:   `kind` says what the swatch previews: "tint" shows the solid hue (a
//        chart's line), "background" the gradient the tile would wear. A stored
//        value the registry does not list (a legacy `series-1`) still previews
//        as its colour and is replaced when another swatch is chosen.
// WHERE: ./TileInspector.tsx, ./ConditionsEditor.tsx, ./tiles.ts

interface PickerProps {
  value: string | null;
  onChange: (next: string | null) => void;
  kind: "tint" | "background";
  /// Accessible name, e.g. "Line colour".
  label: string;
}

function preview(value: string | null, kind: "tint" | "background"): string | undefined {
  if (kind === "background") return value === null ? undefined : gradientCss(value);
  return tintCss(value);
}

/// "Default" for a tint left to the series colours (none, or a legacy `series-N`), "None" for no background.
function pickerLabel(value: string | null, kind: "tint" | "background"): string {
  if (isSwatch(value)) return swatchLabel(value);
  return kind === "tint" ? "Default" : "None";
}

export function TileColorPicker({ value, onChange, kind, label }: PickerProps) {
  const current = preview(value, kind);
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm" aria-label={`${label}: ${pickerLabel(value, kind)}`} className="gap-2 font-normal">
          <span className={cn("size-3.5 rounded-full border border-border", current === undefined ? "bg-surface-secondary" : "")} style={current === undefined ? undefined : { background: current }} />
          <span className="text-xs">{pickerLabel(value, kind)}</span>
          <Icon name="chevron-down" size={12} className="text-muted" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-2" align="start">
        <div className="grid grid-cols-8 gap-1.5">
          <button
            type="button"
            aria-label={kind === "tint" ? "Default colour" : "No colour"}
            title={kind === "tint" ? "Default" : "None"}
            className={cn("flex size-5 items-center justify-center rounded-full border border-border bg-surface-secondary text-muted", !isSwatch(value) ? "ring-2 ring-accent" : "hover:ring-1 hover:ring-border")}
            onClick={() => { onChange(null); }}
          >
            <Icon name="x" size={11} />
          </button>
          {TILE_SWATCHES.map((s) => (
            <button
              key={s.name}
              type="button"
              aria-label={s.label}
              title={s.label}
              className={cn("size-5 rounded-full border border-border", value === s.name ? "ring-2 ring-accent ring-offset-1 ring-offset-overlay" : "hover:ring-1 hover:ring-border")}
              style={{ background: preview(s.name, kind) }}
              onClick={() => { onChange(s.name); }}
            />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}
