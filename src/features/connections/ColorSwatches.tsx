// SOT: connection-colour-picker
import type { ConnectionColor } from "@/lib/bindings";
import { CONNECTION_COLOR_ORDER, connectionColorMeta } from "@/lib/connectionGroups";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";

// WHAT:  One row of colour swatches plus "none" for a connection's colour tag.
// HOW:   Swatches are the app's Button with a token fill; the chosen one gets a
//        ring and a check, so the choice does not rely on colour alone.
// WHERE: src/lib/connectionGroups.ts (CONNECTION_COLORS)
export function ColorSwatches({ value, onChange }: { value: ConnectionColor | null; onChange: (color: ConnectionColor | null) => void }) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label>Colour</Label>
      <div role="radiogroup" aria-label="Colour" className="flex flex-wrap items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          role="radio"
          aria-checked={value === null}
          aria-label="No colour"
          onClick={() => onChange(null)}
          className={cn("size-7 rounded-full border border-border p-0 text-muted", value === null ? "ring-2 ring-ring" : "")}
        >
          <Icon name="x" size={12} />
        </Button>
        {CONNECTION_COLOR_ORDER.map((color) => {
          const meta = connectionColorMeta(color);
          const selected = value === color;
          return (
            <Button
              key={color}
              variant="ghost"
              size="sm"
              role="radio"
              aria-checked={selected}
              aria-label={meta.label}
              title={meta.label}
              onClick={() => onChange(color)}
              className={cn("size-7 rounded-full p-0", selected ? "ring-2 ring-ring" : "")}
            >
              <span aria-hidden className={cn("flex size-5 items-center justify-center rounded-full text-background", meta.fill)}>
                {selected ? <Icon name="check" size={11} /> : null}
              </span>
            </Button>
          );
        })}
      </div>
    </div>
  );
}
