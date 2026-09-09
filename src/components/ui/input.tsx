// SOT: input, text-input, search-input
import type * as React from "react";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Input, styled as this app's glass field.
// WHERE: https://ui.shadcn.com/docs/components/input
export function Input({ className, type = "text", ...props }: React.ComponentProps<"input">) {
  return (
    <input
      data-slot="input"
      type={type}
      className={cn(
        "flex h-8 w-full min-w-0 rounded-[var(--field-radius)] border border-input bg-field px-2.5 py-1 text-[13px] text-field-foreground",
        "shadow-[var(--field-shadow)] outline-none transition-[color,box-shadow,border-color]",
        "placeholder:text-field-placeholder",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "aria-invalid:border-danger aria-invalid:ring-danger/30",
        "file:h-7 file:border-0 file:bg-transparent file:text-xs file:font-medium",
        className,
      )}
      {...props}
    />
  );
}

export interface SearchInputProps extends Omit<React.ComponentProps<"input">, "value" | "onChange" | "type"> {
  value: string;
  onChange: (value: string) => void;
}

// WHAT:  Input with a leading magnifier and a clear button that appears once
//        there is something to clear.
// WHY:   Eight panels filter a long list (tables, objects, engines, commands);
//        they were all rebuilding the same three parts around a bare input.
// HOW:   `type="search"` for the Escape-to-clear browser behaviour, with the
//        native decoration removed so the button below is the only affordance.
export function SearchInput({ className, value, onChange, placeholder = "Search…", ...props }: SearchInputProps) {
  return (
    <div data-slot="search-input" className="relative flex w-full items-center">
      <Icon name="search" aria-hidden className="pointer-events-none absolute left-2 size-3.5 text-muted" />
      <Input
        type="search"
        value={value}
        onChange={(event) => { onChange(event.target.value); }}
        placeholder={placeholder}
        className={cn("px-7 [&::-webkit-search-cancel-button]:appearance-none", className)}
        {...props}
      />
      {value.length > 0 ? (
        <button
          type="button"
          aria-label="Clear search"
          onClick={() => { onChange(""); }}
          className="absolute right-1.5 grid size-5 place-items-center rounded text-muted transition-colors hover:bg-surface-secondary hover:text-foreground"
        >
          <Icon name="x" className="size-3" />
        </button>
      ) : null}
    </div>
  );
}
