// SOT: toolbar-overflow, overflow-menu, more-menu, responsive-toolbar, toolbar-actions
import { Fragment } from "react";
import type { ShortcutAction } from "@/lib/keymap";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Shortcut } from "./Kbd";

/// One command a toolbar or a "⋯" menu can run.
export interface ToolbarAction {
  id: string;
  label: string;
  icon: IconName;
  onSelect: () => void;
  disabled?: boolean | undefined;
  /// Toggle state (a panel that is open, a mode that is on).
  active?: boolean | undefined;
  /// Destructive entries read in the danger colour inside the menu.
  danger?: boolean | undefined;
  /// The live chord, shown in the tooltip and beside the menu entry.
  shortcut?: ShortcutAction | undefined;
  /// Where it lives:
  ///   always — in the bar at every width (Run, Refresh, Search: the 2–4 a view is for);
  ///   wide   — in the bar when the container is wide enough, otherwise in "⋯";
  ///   menu   — only in "⋯" (occasional and destructive commands).
  priority?: "always" | "wide" | "menu" | undefined;
  /// Menu entries with a different group get a separator between them.
  group?: string | undefined;
}

/// Container widths (of the nearest `@container` ancestor) a toolbar folds at.
/// Tailwind's named container sizes: xs 20rem · sm 24rem · md 28rem · lg 32rem ·
/// xl 36rem · 2xl 42rem · 3xl 48rem · 4xl 56rem.
export type FoldWidth = "xs" | "sm" | "md" | "lg" | "xl" | "2xl" | "3xl" | "4xl";

// Literal class strings, so Tailwind's scanner sees every one of them.
const SHOW_FROM: Record<FoldWidth, string> = {
  xs: "hidden @xs:flex",
  sm: "hidden @sm:flex",
  md: "hidden @md:flex",
  lg: "hidden @lg:flex",
  xl: "hidden @xl:flex",
  "2xl": "hidden @2xl:flex",
  "3xl": "hidden @3xl:flex",
  "4xl": "hidden @4xl:flex",
};
const HIDE_FROM: Record<FoldWidth, string> = {
  xs: "flex @xs:hidden",
  sm: "flex @sm:hidden",
  md: "flex @md:hidden",
  lg: "flex @lg:hidden",
  xl: "flex @xl:hidden",
  "2xl": "flex @2xl:hidden",
  "3xl": "flex @3xl:hidden",
  "4xl": "flex @4xl:hidden",
};
const LABEL_FROM: Record<FoldWidth, string> = {
  xs: "hidden @xs:inline",
  sm: "hidden @sm:inline",
  md: "hidden @md:inline",
  lg: "hidden @lg:inline",
  xl: "hidden @xl:inline",
  "2xl": "hidden @2xl:inline",
  "3xl": "hidden @3xl:inline",
  "4xl": "hidden @4xl:inline",
};

interface OverflowMenuProps {
  items: readonly ToolbarAction[];
  /// Accessible name and tooltip of the trigger.
  label?: string;
  icon?: IconName;
  align?: "start" | "center" | "end";
  className?: string | undefined;
}

// WHAT:  The "⋯" menu: every command a toolbar has no room for, or that is
//        too occasional to earn a button.
// WHY:   Folding into a menu, never clipping, is the rule for every toolbar:
//        a command that overflows the edge is a command that no longer exists.
// HOW:   Radix DropdownMenu with an explicit trigger (the app's Button, asChild).
//        Entries are grouped by `group`, with a separator between groups.
// WHERE: docs/ui-refresh.md ("Toolbar overflow")
export function OverflowMenu({ items, label = "More actions", icon = "more", align = "end", className }: OverflowMenuProps) {
  if (items.length === 0) return null;
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label={label} className={cn("rounded-md liquid-hover", className)}>
              <Icon name={icon} size={14} />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align={align} className="min-w-48">
        {items.map((item, index) => {
          const previous = items[index - 1];
          const separate = previous !== undefined && (previous.group ?? "") !== (item.group ?? "");
          return (
            <Fragment key={item.id}>
              {separate ? <DropdownMenuSeparator /> : null}
              <DropdownMenuItem
                variant={item.danger === true ? "danger" : "default"}
                disabled={item.disabled === true}
                onSelect={() => { item.onSelect(); }}
              >
                <Icon name={item.icon} size={13} className={item.danger === true ? "" : item.active === true ? "text-accent" : "text-muted"} />
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                {item.active === true ? <Icon name="check" size={12} className="text-accent" /> : null}
                {item.shortcut !== undefined ? (
                  <DropdownMenuShortcut>
                    <Shortcut action={item.shortcut} />
                  </DropdownMenuShortcut>
                ) : null}
              </DropdownMenuItem>
            </Fragment>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

interface ToolbarButtonProps {
  action: ToolbarAction;
  /// Show the text label beside the icon from this container width up.
  labelFrom?: FoldWidth | undefined;
}

// WHAT:  One toolbar command as an icon button; the label lives in its tooltip
//        (and, on a wide toolbar, beside the icon).
export function ToolbarButton({ action, labelFrom }: ToolbarButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={action.label}
          aria-pressed={action.active}
          variant={action.active === true ? "secondary" : "ghost"}
          size={labelFrom === undefined ? "icon-sm" : "sm"}
          disabled={action.disabled === true}
          onClick={() => { action.onSelect(); }}
          className={cn(
            "rounded-md liquid-hover",
            labelFrom === undefined ? "" : "min-w-6 gap-1 px-1.5 text-xs font-normal",
            action.active === true ? "glass-pill border-0 text-accent" : "",
            action.danger === true ? "text-danger hover:text-danger" : "",
          )}
        >
          <Icon name={action.icon} size={14} />
          {labelFrom !== undefined ? <span className={LABEL_FROM[labelFrom]}>{action.label}</span> : null}
        </Button>
      </TooltipTrigger>
      <TooltipContent className="flex items-center gap-1.5">
        {action.label}
        {action.shortcut !== undefined ? <Shortcut action={action.shortcut} /> : null}
      </TooltipContent>
    </Tooltip>
  );
}

interface ToolbarActionsProps {
  actions: readonly ToolbarAction[];
  /// Below this container width the `wide` actions move into "⋯".
  foldBelow?: FoldWidth;
  /// Show text labels beside the icons from this container width up (wide
  /// work surfaces: the grid toolbar, the query toolbar). Omit for icons only.
  labelsFrom?: FoldWidth | undefined;
  menuLabel?: string;
  className?: string | undefined;
}

// WHAT:  A toolbar's command cluster that folds into "⋯" as its container narrows.
// WHY:   Panels here resize (sidebar 180–520 px, split panes, inspector); a row
//        of buttons sized for one width clips at another. Container queries
//        key the fold to the toolbar's own width, not the window's.
// HOW:   The caller's row carries `@container`. `always` actions render as
//        buttons; `wide` ones render twice — as buttons shown from `foldBelow`
//        up, and as entries in a "⋯" menu shown below it; `menu` ones live in
//        the menu only. Two menus (narrow and wide) because a portalled menu
//        cannot read the toolbar's container width.
// WHERE: docs/ui-refresh.md ("Toolbar overflow"), src/features/shell/TablesPanel.tsx
export function ToolbarActions({ actions, foldBelow = "sm", labelsFrom, menuLabel = "More actions", className }: ToolbarActionsProps) {
  const always = actions.filter((a) => (a.priority ?? "always") === "always");
  const wide = actions.filter((a) => a.priority === "wide");
  const menuOnly = actions.filter((a) => a.priority === "menu");
  const narrowMenu = [...wide, ...menuOnly];
  return (
    <span className={cn("flex shrink-0 items-center gap-0.5", className)}>
      {always.map((a) => (
        <ToolbarButton key={a.id} action={a} labelFrom={labelsFrom} />
      ))}
      {wide.length > 0 ? (
        <span className={cn("items-center gap-0.5", SHOW_FROM[foldBelow])}>
          {wide.map((a) => (
            <ToolbarButton key={a.id} action={a} labelFrom={labelsFrom} />
          ))}
        </span>
      ) : null}
      {narrowMenu.length > 0 ? (
        <span className={HIDE_FROM[foldBelow]}>
          <OverflowMenu items={narrowMenu} label={menuLabel} />
        </span>
      ) : null}
      {menuOnly.length > 0 ? (
        <span className={SHOW_FROM[foldBelow]}>
          <OverflowMenu items={menuOnly} label={menuLabel} />
        </span>
      ) : null}
    </span>
  );
}
