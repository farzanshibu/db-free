// SOT: table-toolbar-menu, table-more-menu, grid-export-menu, auto-refresh-menu
import type { ExportFormat } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { copyLabel } from "./copyFormats";
import { TriggerTip } from "./toolLabel";

/// One "Copy … as" / "Download … as" group of the export menu.
export interface ExportMenuGroup {
  id: string;
  label: string;
  formats: readonly ExportFormat[];
  run: (format: ExportFormat) => void;
}

// WHAT:  The export entries (copy / download page / download all, each by
//        format, then "Export with options…"), shared by the toolbar's Export
//        dropdown and the "⋯" menu's Export submenu.
// WHY:   On a narrow toolbar the Export button folds into "⋯"; both places must
//        offer exactly the same commands.
export function ExportMenuItems({ groups, onOptions }: { groups: readonly ExportMenuGroup[]; onOptions: () => void }) {
  return (
    <DropdownMenuGroup>
      {groups.map((group) => (
        <DropdownMenuSub key={group.id}>
          <DropdownMenuSubTrigger>{group.label}</DropdownMenuSubTrigger>
          <DropdownMenuPortal>
            <DropdownMenuSubContent className="min-w-40">
              {group.formats.map((format) => (
                <DropdownMenuItem key={format} textValue={`${group.label} ${copyLabel(format)}`} onSelect={() => { group.run(format); }}>
                  <span>{copyLabel(format)}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
      ))}
      <DropdownMenuSeparator />
      <DropdownMenuItem textValue="Export with options" onSelect={onOptions}>
        <span>Export with options…</span>
      </DropdownMenuItem>
    </DropdownMenuGroup>
  );
}

interface TableToolbarMenuProps {
  /// The narrow copy of the menu also carries the commands that fold off the bar.
  narrow: boolean;
  className?: string | undefined;
  view: "grid" | "cards";
  onView: (view: "grid" | "cards") => void;
  autoRefresh: number;
  intervals: readonly number[];
  intervalLabel: (seconds: number) => string;
  onAutoRefresh: (seconds: number) => void;
  exportGroups: readonly ExportMenuGroup[];
  exportDisabled: boolean;
  onExportOptions: () => void;
  /// Object storage: byte-exact download of the selected file.
  download?: { disabled: boolean; onSelect: () => void } | undefined;
  /// Rows staged for deletion when chosen; 0 = nothing selected.
  deleteCount: number;
  onDelete?: (() => void) | undefined;
}

// WHAT:  The table tab's "⋯": card view, auto-refresh, delete selected rows and,
//        on a narrow toolbar, Export (and Download for object storage).
// WHY:   The grid toolbar shows the commands the view is for (Insert, Refresh,
//        Filter, Sort, Columns); the occasional ones and the destructive one live
//        here, so the bar never scrolls or clips. A flat OverflowMenu cannot hold
//        the export-format and interval submenus, hence a local menu.
// HOW:   Rendered twice by TableTab — a narrow copy shown below the toolbar's fold
//        width and a wide copy above it — because a portalled menu cannot read
//        the toolbar's container width (same trick as ToolbarActions).
// WHERE: src/features/grid/TableTab.tsx, src/components/global/Toolbar.tsx
export function TableToolbarMenu(props: TableToolbarMenuProps) {
  const { narrow, className, view, onView, autoRefresh, intervals, intervalLabel, onAutoRefresh, exportGroups, exportDisabled, onExportOptions, download, deleteCount, onDelete } = props;
  return (
    <DropdownMenu>
      <TriggerTip tip="More table actions">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label="More table actions" className={cn("rounded-md liquid-hover", className)}>
            <Icon name="more" size={14} />
          </Button>
        </DropdownMenuTrigger>
      </TriggerTip>
      <DropdownMenuContent align="end" className="min-w-52 glass-modal rounded-xl">
        {narrow ? (
          <>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger disabled={exportDisabled} className="data-[disabled]:opacity-50">
                <Icon name="download" size={13} className="text-muted" />
                Export
              </DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent className="min-w-52">
                  <ExportMenuItems groups={exportGroups} onOptions={onExportOptions} />
                </DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>
            {download !== undefined ? (
              <DropdownMenuItem disabled={download.disabled} onSelect={download.onSelect}>
                <Icon name="download" size={13} className="text-muted" />
                Download file
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
          </>
        ) : null}
        <DropdownMenuItem textValue="Card view" onSelect={() => { onView(view === "cards" ? "grid" : "cards"); }}>
          <Icon name="grid" size={13} className={view === "cards" ? "text-accent" : "text-muted"} />
          <span className="flex-1">Card view</span>
          {view === "cards" ? <Icon name="check" size={12} className="text-accent" /> : null}
        </DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <Icon name="clock" size={13} className={autoRefresh > 0 ? "text-accent" : "text-muted"} />
            <span className="flex-1">Auto-refresh</span>
            <span className="text-[11px] text-muted">{autoRefresh > 0 ? intervalLabel(autoRefresh) : "Off"}</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuPortal>
            <DropdownMenuSubContent className="min-w-40">
              {intervals.map((seconds) => (
                <DropdownMenuItem key={seconds} textValue={intervalLabel(seconds)} onSelect={() => { onAutoRefresh(seconds); }}>
                  <span className="flex-1">{intervalLabel(seconds)}</span>
                  {autoRefresh === seconds ? <Icon name="check" size={13} className="ml-2 text-accent" /> : null}
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuPortal>
        </DropdownMenuSub>
        {onDelete !== undefined ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="danger" disabled={deleteCount === 0} onSelect={onDelete}>
              <Icon name="trash" size={13} />
              {deleteCount > 0 ? `Delete ${deleteCount} selected ${deleteCount === 1 ? "row" : "rows"}` : "Delete selected rows"}
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
