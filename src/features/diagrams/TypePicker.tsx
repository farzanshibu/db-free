// SOT: column-type-picker, type-combobox, designer-type-field
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";

interface TypePickerProps {
  value: string;
  options: readonly string[];
  onChange: (type: string) => void;
  ariaLabel: string;
}

// WHAT:  A text field for a column's type with a list of the engine's usual types.
// WHY:   The list is a suggestion, not a whitelist: engines have more types and
//        extensions than any list can carry, so the field stays typeable and the
//        chevron opens the suggestions for whoever would rather pick.
export function TypePicker({ value, options, onChange, ariaLabel }: TypePickerProps) {
  const [open, setOpen] = useState(false);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <div className="relative">
        <Input
          value={value}
          onChange={(event) => {
            onChange(event.target.value);
          }}
          aria-label={ariaLabel}
          spellCheck={false}
          className="h-6 pr-7 font-mono text-xs"
        />
        <PopoverTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label="Suggested types" className="absolute top-0.5 right-0.5 size-5 min-w-5">
            <Icon name="chevron-down" size={12} />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent align="end" className="w-56 p-1">
        <ScrollArea className="max-h-60">
          {options.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => {
                onChange(option);
                setOpen(false);
              }}
              className={cn(
                "flex w-full items-center rounded-md px-2 py-1 text-left font-mono text-xs hover:bg-surface-secondary",
                option.toLowerCase() === value.trim().toLowerCase() ? "bg-surface-secondary text-accent" : "text-foreground",
              )}
            >
              {option}
            </button>
          ))}
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
