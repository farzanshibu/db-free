// SOT: toolbar-menu, icon-dropdown-button, diagram-toolbar-dropdown
import { Fragment } from "react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";

export interface ToolbarMenuItem {
  id: string;
  label: string;
  icon?: IconName;
  disabled?: boolean;
  /// Marks the current choice with a check.
  selected?: boolean;
  /// Rendered as a heading above this item (starts a new section).
  section?: string;
}

interface ToolbarMenuProps {
  icon: IconName;
  label: string;
  items: readonly ToolbarMenuItem[];
  onSelect: (id: string) => void;
  disabled?: boolean;
  /// Draws the button as pressed (a mode that is on).
  active?: boolean;
  /// Show the label next to the icon instead of only in the tooltip.
  showLabel?: boolean;
}

// WHAT:  An icon button that opens a menu of choices (layout direction, export format).
// WHY:   The shared IconButton is a plain button with a tooltip; a menu trigger must
//        be the Radix trigger itself, and the tooltip has to wrap the trigger, so
//        the pair is composed here once for both diagram toolbars.
export function ToolbarMenu({ icon, label, items, onSelect, disabled = false, active = false, showLabel = false }: ToolbarMenuProps) {
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={label}
              variant={active ? "secondary" : "ghost"}
              size={showLabel ? "sm" : "icon-sm"}
              disabled={disabled}
              className={cn("rounded-lg liquid-hover", active ? "glass-pill border-0 text-accent" : "")}
            >
              <Icon name={icon} size={showLabel ? 13 : 14} />
              {showLabel ? label : null}
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="min-w-48">
        {items.map((item, index) => (
          <Fragment key={item.id}>
            {item.section !== undefined ? (
              <>
                {index > 0 ? <DropdownMenuSeparator /> : null}
                <DropdownMenuLabel>{item.section}</DropdownMenuLabel>
              </>
            ) : null}
            <DropdownMenuItem
              disabled={item.disabled ?? false}
              onSelect={() => {
                onSelect(item.id);
              }}
            >
              {item.icon !== undefined ? <Icon name={item.icon} size={13} className="shrink-0 text-muted" /> : null}
              <span className="truncate">{item.label}</span>
              {item.selected === true ? <Icon name="check" size={13} className="ml-auto shrink-0 text-accent" /> : null}
            </DropdownMenuItem>
          </Fragment>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
