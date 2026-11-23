# UI refresh — the guide

Phase 1 (branch `feat/ui-refresh-foundation`) set the rules below, added the shared
primitives, and applied them to the shell, Home, the palette and the Settings frame.
Area passes A, B and C start from that branch and follow this page.

The asks: **less text on screen, help in tooltips; not everything at once; use the full
width.** Everything else here is how.

## 1. Rules

### Text budget per view
- **Labels, not sentences.** A control gets a 1–4 word label. A view header gets a title
  and at most one short fact (`5 saved`, `v0.11.0`), never a sentence.
- **≤ 1 visible sentence per view region** (panel, card, dialog body). Count it. Everything
  past that sentence moves to a tip, a disclosure or the docs.
- **No "how to" text in the resting state.** "Right-click a table and choose…", "Use ⌘↵ to…"
  go in a tip. Shortcuts go in tooltips (`shortcut` prop), not in copy.
- **Stay inline (never hide):** destructive / irreversible warnings ("This cannot be undone"),
  live validation errors, read-only / production state, anything the user must see *before*
  pressing the button. Confirm dialogs keep their description.

### Tooltip vs inline text
| Text is… | Put it in |
|---|---|
| The name of an icon-only control | `IconButton` / `ToolbarButton` tooltip (`label`, plus `shortcut`) |
| Why a setting/field exists, what it affects, defaults, formats | `tip` on `Field` / `AppSelect` / `Toggle` / `NumberInput` / `DateTimeField`, or `SettingsRow body` (it renders as a tip) |
| Help for a section, page or empty state | `InfoTip` / `WithTip` beside the heading; `PageHeader tip`; `EmptyState tip` |
| A warning, error or state the user must see first | Inline text. Not a tip. |
| Long reference prose (per-node help, engine notes) | First sentence inline (or a tip), the rest in a `Disclosure label="More"` |

Never use the native `title=""` attribute for help: it is slow, unstyled and invisible to
keyboard users. Use `Tooltip`/`InfoTip`. (`title` on a truncated name, to show the full name,
is fine.)

### When to collapse into "More" / "Advanced"
- A form or settings group with **> 6 controls**: keep the 3–5 that most people change; put
  the rest in `<Disclosure label="Advanced" count={n}>`.
- **Rarely changed or expert options** (driver params, TLS files, timeouts, SRV, retries,
  "allow private network") always go under Advanced, closed by default.
- **Optional extras of a common task** ("filter", "headers", "limit") go under
  `<Disclosure variant="inline" label="More options">`.
- **Open it for them** when a hidden field has a validation error or a non-default value
  (controlled `open` / `onOpenChange`).
- Collapse, never delete: every hidden control stays reachable through a disclosure, a "⋯"
  menu, a tooltip or the command palette.

### Full-width layout
- **No fixed max-width centre column on a work surface.** Grids, editors, lists, card boards,
  forms with a preview, settings: the body spans the window (`Page width="full"`, the default).
- **Only running prose is capped** (`Page width="readable"`, 80ch): release notes, long help.
- **Cards and tiles use `AutoGrid`** (`min="xs|sm|md|lg"` = 12/16/20/26rem): as many columns as
  fit. Not `grid-cols-2`, not `flex-wrap` with `w-40` tiles.
- **Side panels are resizable and remembered:** `SplitLayout` (`side`, `defaultSize`, `min`,
  `max`, `storageKey`). Not `w-64 shrink-0`. Applies to list/inspector panes next to a work
  surface (tools, transfer, compare, diagram inspector, pending changes).
- **Settings-like row stacks** flow into columns: the Settings frame does this for every
  `SettingsRow` (see `SettingsPage.tsx`); other row stacks use `AutoGrid min="lg"`.
- Dialogs keep a sensible max width (they are modal, not surfaces); a dialog that holds a
  work surface (export preview, versions diff) uses `w-[min(92vw,…)]`.

### Toolbar overflow
- A toolbar shows the **2–4 commands the view is for** (`priority: "always"`), the next few
  while there is room (`"wide"`), and the rest in "⋯" (`"menu"`). Destructive commands live
  in the menu.
- **Fold, never clip or scroll.** The toolbar row gets `@container`; `ToolbarActions
  foldBelow="sm|md|…"` keys the fold to the toolbar's own width, so it works in a narrow
  sidebar, a split pane and a full window alike.
- Wide work surfaces may show text labels beside icons from a width up (`labelsFrom="3xl"`).
- One "⋯" per toolbar, at its right end.

### Empty states
- `EmptyState`: **icon + title (2–4 words) + the one action that fixes it** (+ at most one
  secondary). Explanation goes in `tip`. `body` only for a sentence needed to act.
- `size="inline"` inside panels and lists; `size="page"` for a whole tab or page.
- A list's "no results" for a search is one line, no icon (`No tables match.`).

### Density
- Chrome spacing comes from the density tokens (`app-titlebar`, `app-toolbar`, `app-pad`,
  `app-pad-x`, `app-row`, `app-stack` in `globals.css`); don't hard-code row heights or panel
  padding for chrome.
- Type scale: 13px titles, 12px (`text-xs`) body and controls, 11px meta, 10px badges only.
- Use tokens only (`bg-surface`, `text-muted`, `border-border`…). Both themes must work — check
  light (`data-theme="light"`) as well as dark.

## 2. Primitives (where, and how to call them)

| Need | Use | File |
|---|---|---|
| Help icon + tooltip | `<InfoTip label="About X">text</InfoTip>`; heading + tip: `<WithTip tip="…">…</WithTip>` | `components/global/InfoTip.tsx` |
| Field label with tip | `tip` prop on `Field`, `AppSelect`, `Toggle`, `NumberInput`, `DateTimeField`; bare: `FieldLabel` | `components/global/Field.tsx` |
| Advanced / More | `<Disclosure label="Advanced" count={4} tip="…">…</Disclosure>`, `variant="inline"` for "More options" | `components/global/Disclosure.tsx` (+ vendored `ui/collapsible.tsx`) |
| "⋯" menu | `<OverflowMenu items={ToolbarAction[]} label="…" />` (groups → separators, `danger`, `shortcut`, `active`) | `components/global/Toolbar.tsx` |
| Toolbar that folds | row `className="@container …"` + `<ToolbarActions foldBelow="sm" actions={[…priority…]} />` | `components/global/Toolbar.tsx` |
| Icon button with shortcut | `<IconButton … shortcut="new-query" tooltipSide="right" />` | `components/global/Button.tsx` |
| Page header | `<PageHeader title onBack tip meta actions />` (drag region handled) | `components/global/Layout.tsx` |
| Full page | `<Page header={…} width="full"|"readable" scroll>` | `components/global/Layout.tsx` |
| Resizable side panel | `<SplitLayout panel={…} side="right" defaultSize={320} storageKey="db-free:x-width">main</SplitLayout>` | `components/global/Layout.tsx` |
| Card grid | `<AutoGrid min="sm">…</AutoGrid>` | `components/global/Layout.tsx` |
| Empty state | `<EmptyState icon title tip action size="inline" />` | `components/global/EmptyState.tsx` |
| Splitter | `Resizer` now has `label` and arrow-key resizing (Shift = 64px) | `components/global/Resizer.tsx` |

Reference implementation (phase 1): `features/shell/{IconRail,TabBar,TabView,TablesPanel,UpdateBar,WhatsNewPage,Sidebar,PaneArea}.tsx`,
`features/home/{HomeBoard,PinnedBoard,RecentsPanel}.tsx`, `features/palette/CommandPalette.tsx`,
`features/settings/{SettingsPage (frame only),SettingsRow}.tsx`.

What phase 1 already changed globally: every `SettingsRow body` is now a tip (all sections), and
the Settings body is a fluid multi-column grid (rows are cells; any other child spans the row).
Area B: check each section still reads well in 2–3 columns, and wrap a block that must stay
one column in a single element (it then spans the full row).

## 3. Per-feature checklist

Worst offenders found by a scan of `src/features/*` (long inline prose, centred columns, fixed
panel widths, scrolling toolbars, native `title=` help). Line numbers are at phase-1 HEAD.
Tick each, then sweep the rest of the folder with the rules above.

### Group A — editor, grid, queries, history, changes
- **grid**
  - [ ] `grid/TableTab.tsx:1022` toolbar is a horizontally *scrolling* strip → `@container` + `ToolbarActions` (Insert/Refresh/Filter always; Sort/Columns/Export wide; Delete, Auto, Cards… in "⋯"), `labelsFrom="3xl"`.
  - [ ] `grid/FilterBar.tsx:105` placeholder carries a whole example query + chord → short placeholder (`Filter…`), example + chord in an `InfoTip`.
  - [ ] `grid/FilterPopover.tsx:93` (`w-[480px]`), `grid/SortPopover.tsx:54` (`w-[420px]`), `grid/ColumnsPopover.tsx:34` → `w-[min(…,90vw)]`; long helper lines → tips.
  - [ ] `grid/InsertRowModal.tsx:51,57` fixed `w-[520px]` + `grid-cols-2` → `AutoGrid min="sm"` inside a wider dialog; nullable/default hints as field `tip`.
  - [ ] `grid/RecordInspector.tsx:84` — inspector width already stored; adopt `SplitLayout` semantics (arrow-key resize via `Resizer label`).
- **editor**
  - [ ] `editor/QueryPane.tsx:646` toolbar → `ToolbarActions` (Run, Stop always; Format/Explain/Save wide; transactions, open file, params in "⋯").
  - [ ] `editor/QueryPane.tsx:1138` destructive-statement warning — keep inline (rule), but trim to one sentence.
  - [ ] `editor/ResultsPane.tsx:271,351` empty/affected states → `EmptyState size="inline"`; 9 native `title=` → `Tooltip`.
  - [ ] `editor/ResultChart.tsx:72,243` long empty body + `max-w-110` → `EmptyState tip`, full-width chart.
  - [ ] `editor/QueryErrorPanel.tsx:72` `max-w-3xl` → full width; hint/position as tips.
  - [ ] `editor/SaveQueryDialog.tsx:86` fixed `w-[640px]`; folder/version help → field tips.
- **queries**
  - [ ] `queries/QueriesPanel.tsx:299,301,317` three separate "nothing here" sentences → one `EmptyState size="inline"` each with an action.
  - [ ] `queries/QueriesPanel.tsx:391,427,440` dialog descriptions → one sentence (keep the destructive one inline).
  - [ ] `queries/QueryVersionsDialog.tsx:81` `w-[760px]` diff dialog → `w-[min(92vw,1100px)]`, list as `SplitLayout`.
  - [ ] 7 native `title=` in `QueriesPanel.tsx` → `Tooltip`.
- **history**
  - [ ] `history/HistoryTab.tsx:157,159` empty bodies → `tip`; filters row → `@container` fold; retention text (`:192`) stays (destructive).
  - [ ] 3 native `title=` → `Tooltip`.
- **changes**
  - [ ] `changes/PendingChangesPanel.tsx:56-70` hand-rolled resize + raw `localStorage` → `SplitLayout side="right" storageKey="db-free:changes-panel-width"`.
  - [ ] `changes/PendingChangesPanel.tsx:191` `w-[480px]` confirm → standard dialog width; review-mode explanation → `InfoTip`.

### Group B — connections, settings, backup, transfer, keys, admin, engines
- **connections**
  - [ ] `connections/ConnectionsPage.tsx:132` `mx-auto max-w-6xl` centre column on the start screen → full width (`Page`/`PageHeader`); connection cards → `AutoGrid min="md"`.
  - [ ] `connections/ConnectionsPage.tsx:190` empty-state body (engine list sentence) → `tip`; action = New connection.
  - [ ] `connections/ConnectionForm.tsx:261` `max-w-[640px]` form column → two-column `AutoGrid min="md"` of fields; SSH, TLS, options, sign-in → `Disclosure` sections ("Advanced", count).
  - [ ] `connections/ConnectionForm.tsx:397` 200-char SSH/TLS paragraph → `InfoTip`.
  - [ ] `connections/ConnectionOptionsFields.tsx:44,66` `description=` → `tip`; whole block under `Disclosure label="Advanced"`.
  - [ ] `connections/ConnectionPicker.tsx:48` `max-w-[720px]` engine picker → full-width `AutoGrid min="xs"` of engines, category filter as `Segmented`.
  - [ ] `connections/CredentialBanner.tsx:114` long description — destructive, keep inline but one sentence.
  - [ ] 9 native `title=` (cards, chips) → `Tooltip`.
- **settings** (sections; the frame is done)
  - [ ] `settings/AiSection.tsx:366,379` 240–280-char row bodies → split: one-line tip + `Disclosure` for the detail; profiles list → `AutoGrid`.
  - [ ] `settings/SettingsPage.tsx:159` Fonts intro `<p>`, `:258` Command Menu intro, `:283` Inspector Tabs intro → `WithTip` on the `h2`.
  - [ ] `settings/SettingsPage.tsx:259-297` Command Menu / Inspector Tabs reorder lists → compact list inside one full-width card (2 columns of items on wide screens).
  - [ ] `settings/SchedulesSection.tsx:81` 175-char intro → tip on the heading.
  - [ ] `settings/UpdatesSection.tsx` (8 native `title=`), `ThemesSection.tsx` (4), `AiSection.tsx` (5) → `Tooltip`.
  - [ ] `settings/ShortcutsSection.tsx` / `SnippetsSection.tsx` — full-width tables, search on top; move per-row help to tips.
- **backup**
  - [ ] `backup/BackupDialog.tsx:101` fixed `w-[700px]`; `:457-480` three `grid-cols-2` blocks → `AutoGrid min="md"`; tool-path/options → `Disclosure "Advanced"`.
  - [ ] `backup/BackupDialog.tsx:328,479` long notes → keep the overwrite warning inline (one sentence), rest as tips.
- **transfer**
  - [ ] `transfer/ImportPanel.tsx:108` 175-char drop-zone body → title + `tip`.
  - [ ] `transfer/ExportPanel.tsx:129,166` fixed `w-60`/`w-64` side lists → `SplitLayout`; `:238` empty body → `tip`.
  - [ ] `transfer/TableImport.tsx:221,240`, `ExportOptionsForm.tsx:61`, `SqlImport.tsx:153` `description=` → `tip`; options beyond 5 → `Disclosure`.
  - [ ] `transfer/ExportDialog.tsx:159` `w-[800px]` → `w-[min(92vw,1100px)]`.
- **keys**
  - [ ] `keys/KeyTab.tsx:148,165,196` empty/error bodies → `tip` (keep the delete warning inline); `:272` and `keys/TtlEditor.tsx:95` `w-72` → popover width by content.
  - [ ] `keys/CreateKeyDialog.tsx:169,176` fixed width + `grid-cols-2`.
  - [ ] 6 native `title=` → `Tooltip`.
- **admin**
  - [ ] `admin/AdminTab.tsx:140,144` breakpoint grid (`lg:grid-cols-2 2xl:grid-cols-3`) + inner `grid-cols-2` → `AutoGrid min="md"` / `min="xs"`.
  - [ ] `admin/AdminTab.tsx:199` filter row + actions → `@container` fold; 5 native `title=` → `Tooltip`.
- **engines**
  - [ ] `engines/CapabilityMatrixPage.tsx` → `Page`/`PageHeader` (it hand-draws its header); `:126` sticky first column `max-w-[240px]` OK; legend text → `InfoTip`.

### Group C — chat, dashboards, diagrams, documents, workflows, tools, compare, schema, objects, palette, home
- **chat**
  - [ ] `chat/ChatTab.tsx:861` `mx-auto max-w-3xl` composer/suggestions → full width with a readable max only on message *text*.
  - [ ] `chat/ChatHistory.tsx:67` `w-80` history pane → `SplitLayout side="left"`.
  - [ ] `chat/AiWelcomeDialog.tsx:79,86` two-line pitch → one line + tip.
  - [ ] `chat/ModelPicker.tsx:80` `w-80` popover; `chat/Markdown.tsx:224` `w-[520px]`.
- **dashboards**
  - [ ] `dashboards/TileInspector.tsx:210,249` and `ConditionsEditor.tsx:57` `description=` → `tip`; formatting options → `Disclosure "Advanced"`.
  - [ ] `dashboards/DashboardTab.tsx:304` empty body → `tip`; toolbar → `ToolbarActions`. 5 native `title=`.
- **diagrams**
  - [ ] `diagrams/InspectorParts.tsx:28` `w-72` inspector → `SplitLayout side="right"`.
  - [ ] `diagrams/ToolbarMenu.tsx` duplicates `OverflowMenu` → fold into `components/global/Toolbar.tsx`.
  - [ ] `diagrams/ErdTab.tsx:87` empty body → `tip`; 12 native `title=` (TableInspector 6, RelationInspector 4) → `Tooltip`; `TableInspector.tsx:97,235` `grid-cols-2`.
- **documents**
  - [ ] `documents/DocumentsPanel.tsx:299,330` dialog descriptions → one sentence; 7 native `title=`.
  - [ ] `documents/LibraryDialogs.tsx:65,121,279,330` fixed `w-[420–440px]` dialogs — fine as dialogs; trim descriptions.
- **workflows**
  - [ ] `workflows/NodeHelp.tsx:39-124` 170–240-char per-node paragraphs → first sentence + `Disclosure variant="inline" label="More"`.
  - [ ] `workflows/forms/DataForms.tsx:71,76,113,118` long `description=` → `tip`; keep the "can delete data" warning inline (one sentence); network/destructive toggles under `Disclosure "Advanced"`.
  - [ ] `workflows/Inspector.tsx:102` → `tip`; inspector pane → `SplitLayout`.
- **tools**
  - [ ] `tools/ToolShell.tsx:45` `w-72` aside (shared by every tool) → `SplitLayout` — one change fixes all tools.
  - [ ] `tools/XmlViewerTab.tsx:69`, `GraphViewTab.tsx:415`, `SearchTab.tsx:112` fixed `w-60/w-64` asides → `SplitLayout`.
  - [ ] Empty bodies 100–166 chars: `GraphViewTab.tsx:409`, `MessageViewerTab.tsx:131`, `PipelineTab.tsx:190`, `MetricsTab.tsx:130`, `LedgerTab.tsx:72`, `VectorSearchTab.tsx:107` → title + action + `tip`.
  - [ ] 18 native `title=` across tools → `Tooltip`.
- **compare**
  - [ ] `compare/SchemaCompareTab.tsx:112`, `DataCompareTab.tsx:166` 150-char empty bodies → `tip`; `SchemaCompareTab.tsx:116` `w-64` → `SplitLayout`.
- **schema**
  - [ ] `schema/forms/EditTableDialog.tsx:79`, `CreateTableDialog.tsx:58` long descriptions → one line; `CreateTableDialog.tsx:93` `grid-cols-2` → `AutoGrid`; constraints/storage options → `Disclosure "Advanced"`.
  - [ ] `schema/forms/DdlDialog.tsx:49` `max-w-5xl` OK (SQL preview); `ColumnGrid.tsx` 5 native `title=`.
- **objects**
  - [ ] `objects/ObjectTab.tsx:144` empty body → `tip`; `:194` `max-w-xl` detail column → full width `AutoGrid min="md"` of properties; `:215` fixed `w-[480px]`.
- **palette** — done in phase 1 (grouped headings, hints on the highlighted row). Remaining: action `hint` strings in `CommandPalette.tsx` are used for search matching; keep them short.
- **home** — done in phase 1. Its host page is `connections/ConnectionsPage.tsx` (group B, first item).

## 4. Before / after

Screenshots (not in git, `docs/ui-refresh/*.png` is ignored) live in this worktree:
`/Volumes/Vinu1TBSSD/munder-difflin/worktrees/dbfree-ui-foundation/docs/ui-refresh/`
— `{before,after}-{connections,workspace,table,palette,settings,settings-light}.png`.

Take your own with `pnpm preview:ui` (port 1421; the bridge in `scripts/preview-bridge.js` now
stubs window metadata, plugin calls and the newer fixtures). If `node_modules` is a symlink to a
read-only checkout, point Vite's `cacheDir` elsewhere and pass `--configLoader runner`.
