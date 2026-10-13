// SOT: multi-pick, ordered-multi-select, column-multi-picker
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuLabel, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

interface MultiPickProps {
  options: readonly string[];
  /// In the order they were picked: for a composite foreign key the order is the column mapping.
  value: readonly string[];
  onChange: (next: string[]) => void;
  ariaLabel: string;
  placeholder: string;
  disabled?: boolean;
  className?: string;
}

// WHAT:  A compact "pick several" control: a button that shows the picks and a
//        menu of checkable options.
// WHY:   The shadcn Select is single-valued, and a composite foreign key or a
//        ClickHouse ORDER BY needs several columns *in a chosen order*.
// HOW:   Picking appends and unpicking removes, so the list keeps click order. The
//        menu stays open while ticking (`onSelect` is cancelled).
// WHERE: src/features/schema/forms/ForeignKeyEditor.tsx
export function MultiPick({ options, value, onChange, ariaLabel, placeholder, disabled = false, className }: MultiPickProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" disabled={disabled} aria-label={ariaLabel} className={cn("w-full min-w-0 justify-between px-2 font-normal", className)}>
          <span className={cn("truncate text-xs", value.length > 0 ? "font-mono text-foreground" : "text-muted")}>{value.length > 0 ? value.join(", ") : placeholder}</span>
          <Icon name="chevron-down" size={12} className="text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="max-h-64 min-w-48">
        {options.length === 0 ? <DropdownMenuLabel>Nothing to pick yet</DropdownMenuLabel> : null}
        {options.map((option) => (
          <DropdownMenuCheckboxItem
            key={option}
            checked={value.includes(option)}
            onSelect={(event) => {
              event.preventDefault();
            }}
            onCheckedChange={(checked) => {
              onChange(checked ? [...value, option] : value.filter((v) => v !== option));
            }}
          >
            <span className="font-mono text-xs">{option}</span>
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
