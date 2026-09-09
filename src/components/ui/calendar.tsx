// SOT: calendar, day-picker
import type * as React from "react";
import { DayPicker, getDefaultClassNames } from "react-day-picker";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { buttonVariants } from "@/components/ui/button";

// WHAT:  The shadcn/ui Calendar (react-day-picker), themed to the app's palette.
// WHY:   Editing a `date` / `timestamp` cell should not mean typing a format
//        from memory; the grid opens this straight from the cell.
// WHERE: https://ui.shadcn.com/docs/components/calendar
export function Calendar({ className, classNames, showOutsideDays = true, ...props }: React.ComponentProps<typeof DayPicker>) {
  const defaults = getDefaultClassNames();
  return (
    <DayPicker
      showOutsideDays={showOutsideDays}
      className={cn("p-1 text-[13px]", className)}
      classNames={{
        root: cn(defaults.root, "w-fit"),
        // The nav is the absolute layer, pinned across the top; the caption
        // stays in flow and is padded clear of the two arrows. Doing this the
        // other way round floats the month name over the weekday row.
        months: cn(defaults.months, "relative flex flex-col gap-3"),
        month: cn(defaults.month, "flex w-full flex-col gap-3"),
        nav: cn(defaults.nav, "absolute inset-x-0 top-0 flex h-7 w-full items-center justify-between gap-1"),
        button_previous: cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "select-none"),
        button_next: cn(buttonVariants({ variant: "ghost", size: "icon-sm" }), "select-none"),
        month_caption: cn(defaults.month_caption, "flex h-7 w-full items-center justify-center px-7"),
        caption_label: cn(defaults.caption_label, "text-xs font-medium text-foreground"),
        month_grid: cn(defaults.month_grid, "w-full border-collapse"),
        weekdays: cn(defaults.weekdays, "flex"),
        weekday: cn(defaults.weekday, "w-7 text-[10px] font-normal text-muted"),
        week: cn(defaults.week, "mt-0.5 flex w-full"),
        day: cn(defaults.day, "size-7 p-0 text-center"),
        day_button: cn(
          defaults.day_button,
          "size-7 rounded-md text-xs text-foreground outline-none transition-colors",
          "hover:bg-surface-secondary focus-visible:ring-2 focus-visible:ring-ring/50",
        ),
        selected: cn(defaults.selected, "[&>button]:bg-accent [&>button]:text-accent-foreground [&>button]:hover:bg-accent"),
        today: cn(defaults.today, "[&>button]:font-semibold [&>button]:text-accent"),
        outside: cn(defaults.outside, "[&>button]:text-muted/40"),
        disabled: cn(defaults.disabled, "[&>button]:opacity-40"),
        hidden: cn(defaults.hidden, "invisible"),
        ...classNames,
      }}
      components={{
        // react-day-picker asks for one of four directions; the registry has
        // three chevrons, so "up" is the down glyph turned over.
        Chevron: ({ orientation, className: chevronClass }) => (
          <Icon
            name={orientation === "left" ? "chevron-left" : orientation === "right" ? "chevron-right" : "chevron-down"}
            size={14}
            className={cn(orientation === "up" ? "rotate-180" : "", chevronClass)}
          />
        ),
      }}
      {...props}
    />
  );
}
