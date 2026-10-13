// SOT: table-pager, page-jump, page-size-picker, row-count-pill
import { useState } from "react";
import { AppSelect } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatCount } from "@/lib/format";
import { rowTotal } from "./rowCount";
import { PAGE_SIZES } from "./tableState";

interface PagerProps {
  /// 0-based.
  pageIndex: number;
  /// Null while the total is not known yet.
  pageCount: number | null;
  hasNext: boolean;
  onPage: (index: number) => void;
  pageSize: number;
  onPageSize: (size: number) => void;
  total: number | null;
  totalExact: boolean;
  loadedRows: number;
}

const SIZE_OPTIONS = PAGE_SIZES.map((size) => ({ value: String(size), label: `${formatCount(size)} rows` }));

// WHAT:  The pager: previous / next, a page number you can click to jump to, the
//        page-size picker (25 – 1,000) and the row count (842, 1.2K, 3.4M+).
// WHY:   Page 1 of 4,000 needs a way to get to page 3,000 that is not 2,999 clicks.
// HOW:   Clicking "3 / 120" turns it into a number box; Enter goes, Escape leaves.
//        The jump clamps to the pages that exist once the total is known.
// WHERE: src/features/grid/TableTab.tsx
export function Pager({ pageIndex, pageCount, hasNext, onPage, pageSize, onPageSize, total, totalExact, loadedRows }: PagerProps) {
  const [jump, setJump] = useState<string | null>(null);
  const label = rowTotal(total, totalExact, loadedRows);
  const go = () => {
    const wanted = Number(jump);
    setJump(null);
    if (!Number.isInteger(wanted) || wanted < 1) return;
    const last = pageCount ?? Number.POSITIVE_INFINITY;
    onPage(Math.min(wanted, last) - 1);
  };
  return (
    <div className="flex items-center gap-2">
      <div className="flex items-center gap-1 rounded-lg glass-pill px-1.5 py-0.5">
        <IconButton icon="chevron-left" label="Previous page" disabled={pageIndex === 0} onClick={() => onPage(Math.max(0, pageIndex - 1))} size={13} className="size-5 min-w-5" />
        {jump === null ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="xs" aria-label="Go to page" onClick={() => setJump(String(pageIndex + 1))} className="h-5 min-h-5 px-1 font-mono text-[11px] tabular-nums">
                <span className="text-foreground">{formatCount(pageIndex + 1)}</span>
                <span className="text-muted"> / {pageCount === null ? "…" : formatCount(pageCount)}</span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>Go to page</TooltipContent>
          </Tooltip>
        ) : (
          <Input
            autoFocus
            type="number"
            min={1}
            {...(pageCount !== null ? { max: pageCount } : {})}
            value={jump}
            aria-label="Page number"
            onChange={(e) => setJump(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                go();
              } else if (e.key === "Escape") {
                e.preventDefault();
                setJump(null);
              }
            }}
            onBlur={() => setJump(null)}
            className="h-5 w-14 min-w-0 rounded-md px-1 text-center font-mono text-[11px] tabular-nums [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none"
          />
        )}
        <IconButton icon="chevron-right" label="Next page" disabled={!hasNext} onClick={() => onPage(pageIndex + 1)} size={13} className="size-5 min-w-5" />
      </div>
      <AppSelect ariaLabel="Rows per page" value={String(pageSize)} options={SIZE_OPTIONS} size="sm" className="w-28 shrink-0" onChange={(v) => onPageSize(Number(v))} />
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="min-w-14 text-right font-mono text-[11px] tabular-nums text-muted">{label.label} rows</span>
        </TooltipTrigger>
        <TooltipContent>{label.title}</TooltipContent>
      </Tooltip>
    </div>
  );
}
