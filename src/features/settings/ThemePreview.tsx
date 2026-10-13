// SOT: theme-preview, theme-card-preview, scoped-theme-variables
import { useLayoutEffect, useRef } from "react";
import type { ThemeBase } from "@/lib/bindings";
import { ALL_THEME_TOKENS } from "@/lib/themes";
import { isSafeColor } from "@/lib/themeFile";
import { cn } from "@/lib/cn";

// WHAT:  A miniature workbench — rail, sidebar, tabs, a grid, a query, status
//        colours, a chart palette — drawn in a theme's colours, wherever it sits.
// WHY:   Picking a theme from its name tells you nothing; the editor needs a
//        live view of every colour group while you change them, and the gallery
//        needs a thumbnail for each theme.
// HOW:   The colours are written as CSS variables on this element only, so the
//        preview wears the theme while the rest of the app keeps its own. The
//        tokens go on through `style.setProperty` after the same safety check
//        the document gets, and the element's utilities (`bg-surface`,
//        `text-muted`, `glass-sidebar`…) resolve them locally.
// WHERE: src/features/settings/ThemesSection.tsx
export function ThemePreview({ colors, base, compact = false, className }: { colors: Readonly<Record<string, string | undefined>>; base: ThemeBase; compact?: boolean; className?: string }) {
  const box = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const node = box.current;
    if (node === null) return;
    for (const { name } of ALL_THEME_TOKENS) {
      const value = colors[name];
      if (value !== undefined && isSafeColor(value)) node.style.setProperty(name, value);
      else node.style.removeProperty(name);
    }
    node.style.colorScheme = base;
  }, [colors, base]);

  const line = (width: string, tone: string) => <span className={cn("block h-1 rounded-full", tone)} style={{ width }} />;
  return (
    <div
      ref={box}
      aria-hidden
      className={cn("pointer-events-none flex overflow-hidden rounded-lg border border-border text-foreground select-none", compact ? "h-24" : "h-44", className)}
      style={{ background: "var(--page-base)", color: "var(--foreground)" }}
    >
      <div className="glass-dock flex w-5 shrink-0 flex-col items-center gap-1.5 py-1.5">
        <span className="size-2.5 rounded-sm bg-accent" />
        <span className="size-2 rounded-sm bg-muted/50" />
        <span className="size-2 rounded-sm bg-muted/50" />
      </div>
      <div className="glass-sidebar flex w-[26%] shrink-0 flex-col gap-1.5 p-2">
        {line("70%", "bg-foreground/70")}
        {line("55%", "bg-muted/60")}
        {line("62%", "bg-muted/60")}
        {line("48%", "bg-muted/60")}
        {compact ? null : line("58%", "bg-muted/60")}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="glass-header flex items-center gap-1 px-2 py-1">
          <span className="relative rounded-t-sm bg-surface-secondary px-2 py-0.5 text-[8px] after:absolute after:inset-x-1 after:bottom-0 after:h-px after:bg-accent">orders</span>
          <span className="px-2 py-0.5 text-[8px] text-muted">query</span>
          <span className="ml-auto rounded-sm bg-accent px-1.5 py-0.5 text-[8px] font-semibold text-accent-foreground">Run</span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-px bg-background p-1.5 font-mono text-[8px] leading-tight">
          <div className="flex gap-2 rounded-sm bg-surface px-1 py-0.5 text-muted">
            <span className="w-8">id</span>
            <span className="w-12">status</span>
            <span>total</span>
          </div>
          <div className="flex gap-2 px-1 py-0.5">
            <span className="w-8">1</span>
            <span className="w-12" style={{ color: "var(--success)" }}>paid</span>
            <span>42.00</span>
          </div>
          <div className="flex gap-2 px-1 py-0.5" style={{ backgroundColor: "var(--color-selection-strong)" }}>
            <span className="w-8">2</span>
            <span className="w-12" style={{ color: "var(--warning)" }}>due</span>
            <span>19.90</span>
          </div>
          <div className="row-stripe flex gap-2 px-1 py-0.5">
            <span className="w-8">3</span>
            <span className="w-12" style={{ color: "var(--danger)" }}>failed</span>
            <span>7.25</span>
          </div>
          {compact ? null : (
            <>
              <div className="mt-1 rounded-sm bg-surface-secondary px-1.5 py-1">
                <span style={{ color: "var(--color-syntax-keyword)" }}>select</span> <span>id</span>, <span style={{ color: "var(--color-syntax-type)" }}>total</span>{" "}
                <span style={{ color: "var(--color-syntax-keyword)" }}>from</span> <span>orders</span>{" "}
                <span style={{ color: "var(--color-syntax-keyword)" }}>where</span> <span>total</span> <span style={{ color: "var(--color-syntax-operator)" }}>&gt;</span>{" "}
                <span style={{ color: "var(--color-syntax-number)" }}>10</span> <span style={{ color: "var(--color-syntax-comment)" }}>-- big ones</span>
              </div>
              <div className="mt-auto flex items-end gap-1 pt-1">
                {[1, 2, 3, 4, 5, 6, 7].map((n) => (
                  <span key={n} className="w-3 rounded-t-sm" style={{ height: `${String(8 + ((n * 7) % 14))}px`, backgroundColor: `var(--color-series-${String(n)})` }} />
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
