// SOT: page-layout, page-header, split-layout, side-panel, auto-grid, full-width-layout
import { useCallback, useState, type ReactNode } from "react";
import { readStored, writeStored } from "@/lib/storage";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Resizer } from "./Resizer";
import { InfoTip } from "./InfoTip";

interface PageHeaderProps {
  title: string;
  /// What this page is for, behind an info icon beside the title.
  tip?: ReactNode;
  /// Shows a Back button before the title.
  onBack?: (() => void) | undefined;
  backLabel?: string;
  /// One short status fact after the title ("5 saved", "v0.11.0"). Not a sentence.
  meta?: ReactNode;
  /// Right-aligned controls: search, the primary action, then "⋯".
  actions?: ReactNode;
  className?: string | undefined;
}

// WHAT:  The strip along the top of a full page (Settings, What's new,
//        Connections, Capabilities): Back, title, one fact, actions.
// WHY:   Every page drew this by hand, each slightly different. It is also the
//        window's drag region, and the empty space between title and actions
//        must stay draggable — one component gets that right once.
// HOW:   `app-titlebar` keeps it the height the macOS traffic lights are
//        aligned to; `@container` lets its actions fold (ToolbarActions).
// WHERE: src/components/global/Toolbar.tsx, src/styles/globals.css (app-titlebar)
export function PageHeader({ title, tip, onBack, backLabel = "Back", meta, actions, className }: PageHeaderProps) {
  return (
    <div className={cn("@container drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-2 border-b border-border/40 glass-header", className)} data-tauri-drag-region>
      {onBack !== undefined ? (
        <Button variant="ghost" size="sm" onClick={onBack} className="-ml-1 rounded-md px-1.5 text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover">
          <Icon name="chevron-left" size={14} />
          {backLabel}
        </Button>
      ) : null}
      <h1 className="truncate text-[13px] font-semibold tracking-tight text-foreground" data-tauri-drag-region>
        {title}
      </h1>
      {tip !== undefined ? <InfoTip label={`About ${title}`} side="bottom">{tip}</InfoTip> : null}
      {meta !== undefined && meta !== null ? <span className="shrink-0 text-xs text-muted" data-tauri-drag-region>{meta}</span> : null}
      <div className="drag-region h-full min-w-4 flex-1" data-tauri-drag-region />
      {actions !== undefined ? <div className="flex shrink-0 items-center gap-1.5">{actions}</div> : null}
    </div>
  );
}

interface PageProps {
  header: ReactNode;
  children: ReactNode;
  /// `full` (default): the body spans the window — every work surface, list
  ///   and card grid. `readable`: a centred column for running prose only
  ///   (release notes, long help); never for forms, grids or lists.
  width?: "full" | "readable";
  /// false when the body manages its own scrolling (split views, editors).
  scroll?: boolean;
  bodyClassName?: string | undefined;
}

// WHAT:  A full-height page: header strip, then a body that uses the window's width.
// WHY:   The old pages centred a 720–1150 px column and left the rest of a wide
//        window empty. A desktop work surface uses the width it is given; only
//        prose is capped, because line length is what makes prose readable.
export function Page({ header, children, width = "full", scroll = true, bodyClassName }: PageProps) {
  const body = (
    <div className={cn("flex w-full min-w-0 flex-col gap-3 px-4 py-3", width === "readable" ? "mx-auto max-w-[80ch]" : "", bodyClassName)}>{children}</div>
  );
  return (
    <div className="grid-bg relative flex h-full min-h-0 flex-1 flex-col">
      {header}
      {scroll ? <ScrollArea className="min-h-0 flex-1">{body}</ScrollArea> : <div className="flex min-h-0 flex-1">{children}</div>}
    </div>
  );
}

interface SplitLayoutProps {
  /// The resizable side panel (a list, an inspector, a section nav).
  panel: ReactNode;
  /// The main surface; it takes every pixel the panel does not.
  children: ReactNode;
  side?: "left" | "right";
  defaultSize: number;
  min?: number;
  max?: number;
  /// Remembers the width for this viewer (localStorage). Omit for a fixed default.
  storageKey?: string | undefined;
  /// Hides the panel and gives its width to the main surface.
  collapsed?: boolean;
  /// Accessible name of the drag handle ("Resize sidebar").
  resizeLabel?: string;
  className?: string | undefined;
  panelClassName?: string | undefined;
}

function clampSize(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

// WHAT:  A main surface with a resizable panel beside it.
// WHY:   Side panels are where fixed widths hurt most: a 400-table schema and a
//        three-table one want different list widths, and an inspector wants
//        more room on a 27" screen than a laptop. Resizable + remembered is the
//        default for every panel next to a work surface.
// HOW:   The panel width is state, clamped to [min, max]; the existing Resizer
//        (pointer drag and arrow keys) changes it, and the width is written to
//        storage when a drag ends. The main surface is `flex-1 min-w-0`.
// WHERE: src/components/global/Resizer.tsx, docs/ui-refresh.md ("Full-width layout")
export function SplitLayout({ panel, children, side = "left", defaultSize, min = 180, max = 640, storageKey, collapsed = false, resizeLabel = "Resize panel", className, panelClassName }: SplitLayoutProps) {
  const [size, setSize] = useState(() => {
    const stored = storageKey === undefined ? null : Number(readStored(storageKey));
    return clampSize(stored !== null && Number.isFinite(stored) && stored > 0 ? stored : defaultSize, min, max);
  });
  const onResize = useCallback(
    (delta: number) => {
      setSize((current) => clampSize(current + (side === "left" ? delta : -delta), min, max));
    },
    [side, min, max],
  );
  const onDragEnd = useCallback(() => {
    if (storageKey === undefined) return;
    setSize((current) => {
      writeStored(storageKey, String(current));
      return current;
    });
  }, [storageKey]);

  const panelNode = collapsed ? null : (
    <div className={cn("relative flex h-full min-h-0 shrink-0 flex-col", panelClassName)} style={{ width: size }}>
      {panel}
      <Resizer
        direction="horizontal"
        label={resizeLabel}
        onResize={onResize}
        onDragEnd={onDragEnd}
        className={cn("absolute top-0 bottom-0", side === "left" ? "-right-0.5" : "-left-0.5")}
      />
    </div>
  );
  return (
    <div className={cn("flex h-full min-h-0 w-full min-w-0", className)}>
      {side === "left" ? panelNode : null}
      <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">{children}</div>
      {side === "right" ? panelNode : null}
    </div>
  );
}

// Literal class strings, so Tailwind's scanner sees each one.
const MIN_COLUMN = {
  xs: "grid-cols-[repeat(auto-fill,minmax(12rem,1fr))]",
  sm: "grid-cols-[repeat(auto-fill,minmax(16rem,1fr))]",
  md: "grid-cols-[repeat(auto-fill,minmax(20rem,1fr))]",
  lg: "grid-cols-[repeat(auto-fill,minmax(26rem,1fr))]",
} as const;

// WHAT:  A fluid card grid: as many columns as fit, each at least `min` wide.
// WHY:   Replaces fixed `grid-cols-2` / `flex-wrap w-40` card lists, which
//        leave a wide window half empty and squash a narrow one. One rule, no
//        breakpoints: a 1280 px window gets 3–4 columns, a 2560 px one 7–8.
export function AutoGrid({ min = "sm", children, className }: { min?: keyof typeof MIN_COLUMN; children: ReactNode; className?: string | undefined }) {
  return <div className={cn("grid gap-2", MIN_COLUMN[min], className)}>{children}</div>;
}
