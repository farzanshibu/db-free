# DB Pro → DB Free parity roadmap

Source: DB Pro 2.10.1 (`com.dbpro.app`, Electron) unpacked from `Contents/Resources/app.asar`
(renderer `out/renderer/assets/index-*.js`, main `out/main/index.js`, preload `out/preload/index.js`,
app-store migrations `app.asar.unpacked/resources/migrations`). Every row below was checked against
the DB Free code on 2026-10-11 (commit `7f97322`). ✅ parity · 🟡 partial · ❌ missing.
Sizes: S ≤ ½ day, M ≈ 1–2 days, L ≥ 3 days.

**Out of scope** (needs DB Pro's hosted services): sign-in / plans / billing, Studio cloud sync,
presence, activity feed + reactions, public share links, hosted AI tier, telemetry upload,
"Unwrapped" year-in-review, promo offers.

**Where DB Free is already ahead:** 69 engines (DB Pro: 16), a real tool-calling AI agent gated per
statement (DB Pro's chat cannot run anything), SSH host-key pinning, secrets never sent to the UI,
true total row counts when capped, the IN filter operator, range aggregates, fill-paste, FK cell
picker, query parameters, snippets, visual explain plan, result charts, manual transactions, reopen
closed tab, per-step connections in workflows, saved sort/filter per table, schema/data compare,
native backup/restore, object explorer, admin/monitoring, playground tools.

---

## 0. Bugs found during the audit (fix first)

- [ ] **B1** Pooled PG/MySQL connections go back to the pool inside a failed transaction (sqlx 0.8 `return_to_pool` only pings; no ROLLBACK anywhere). `integrations/{postgres,mysql}.rs` — `after_release` ROLLBACK. S
- [ ] **B2** Pending-changes commit sends `BEGIN;…;COMMIT;` as one script; a failure leaves the transaction open. `services/changes.rs` — begin/commit/rollback on one pinned connection; flag 0-row changes. S
- [ ] **B3** Dashboard variables are pasted into SQL unquoted (injection). `features/dashboards/widgetSql.ts` `substituteVars`. S
- [ ] **B4** `run_workflow` passes `confirm_destructive: true`, so workflow steps bypass the destructive-statement confirmation. `commands/workflows.rs`. S
- [ ] **B5** "Delete table…" on a view emits `DROP TABLE`. `features/shell/TablesPanel.tsx`. S
- [ ] **B6** Designer deletions (Backspace / edge removal) never reach `tables`/`relations`, so they are not saved. `features/diagrams/DesignerTab.tsx`. S
- [ ] **B7** `mongodb+srv://` cannot connect (`mongodb.rs` always builds `mongodb://host:port`, `authSource=admin` hard-coded). S
- [ ] **B8** Missing keychain entry silently creates a new master key; old secrets then fail. `adapters/keyring.rs` — detect existing secrets, report "credentials lost". S
- [ ] **B9** Table row count is lost from page 2 (pager shows "…"); Postgres always shows ≈. `services/data.rs`, `TableTab.tsx`. S
- [ ] **B10** Editor theme hardcodes 13px (ignores `--editor-font-size`); `runScope` and `showResultsPane` settings are never read. `features/editor`. S
- [ ] **B11** AI one-shot editor generation (`use_tools:false`) sends no schema though the UI says it does. `services/agent`. S
- [ ] **B12** Connection card right-click Delete has no confirmation. `ConnectionsPage.tsx`. S

---

## 1. Connections & home  (`features/connections`, `commands/connections.rs`, `store/connections.rs`)

- [ ] C1 Connection-string box inside each form (fills fields, engine-mismatch check) S
- [ ] C2 Keep full URI options (`authSource`, `replicaSet`, `options`, `pgbouncer`…), host-based engine detection, auto-SSL for supabase/aws/azure hosts S
- [ ] C3 Copy URL (without password) / Copy details S
- [ ] C4 Duplicate connection "(copy)" incl. sealed secret, SSH, auth S
- [ ] C5 Type-the-name delete confirmation (shared `ConfirmDialog`) S
- [ ] C6 Folders as entities: empty folders, multi-folder membership, delete-confirm, "All | folder" chips M
- [ ] C7 Connection tags: 19-colour palette, presets Production/Staging/Development/Testing/Local, CRUD, duplicate-name check, several per connection, "+N" expander, tag search M
- [ ] C8 Export connections (pick, Select all, "Include passwords" opt-in) → `db-free-connections-YYYY-MM-DD.json`; also reads DB Pro `.dbpro` v1.0 M
- [ ] C9 Import `.json`/`.dbpro` + Navicat `.ncx` (MySQL, MariaDB, PG, MSSQL, Mongo, SQLite, Redis; no DTD/entities); dedupe on engine+host+port+db+user; count + warnings M
- [ ] C10 Open `.db/.sqlite/.sqlite3` from the OS / drag-drop → transient connection with "Save as connection", removed on quit; drop zone on the SQLite form M
- [ ] C11 "Connection failed" dialog with a checklist; Save & connect S
- [ ] C12 Home: pinned board (connection, table, saved query, dashboard, workflow, diagram), drag reorder, unpin, environment badge M
- [ ] C13 Home: recents (6 kinds), 5 chips + full list + Clear; `showPinned` / `showRecents` settings M
- [ ] C14 Sample/demo SQLite database ("Try the demo") S
- [ ] C15 `~/.ssh/config` host import (Host, HostName, User, Port, IdentityFile; skip wildcards) S
- [ ] C16 SSH keyboard-interactive password fallback S
- [ ] C17 Real TLS server name through an SSH tunnel M
- [ ] C18 Custom CA file / client certificate per connection M
- [ ] C19 Per-engine notices in the form (Supabase pooler/IPv4, Neon, ClickHouse, Turso token, D1 ids, PlanetScale branch) S
- [ ] C20 Field-level validation errors S
- [ ] C21 Auth method selector: PG/MySQL password · AWS RDS IAM · Azure AD · GCP Cloud SQL IAM; MSSQL password · Azure AD; Mongo password · Atlas (MONGODB-AWS / OIDC) M
- [ ] C22 AWS RDS IAM token (explicit keys / profile / env chain, region, db user, session token, AssumeRole, RDS CA) M
- [ ] C23 GCP Cloud SQL IAM (ADC / service-account JSON, impersonation) S
- [ ] C24 Azure AD (client secret / managed identity / user+password / access token) M
- [ ] C25 Token refresh 60 s before expiry S
- [ ] C26 DynamoDB: AWS profile, env + default chain, real session-token field, region dropdown M
- [ ] C27 PlanetScale Postgres engine variant; PlanetScale TLS defaults to verify S
- [ ] C28 Open a connection in a new window (setting `openConnectionsInNewWindow`) — see S1
- (deferred) NTLM for MSSQL — tiberius supports Windows auth only on Windows. L

## 2. Data grid, row editing, inspector, pending changes  (`features/grid`, `features/changes`, `ValueEditor.tsx`)

- [ ] G1 Filter: `not contains`, operator list per column type S
- [ ] G2 Filter: All/Any (AND/OR) + per-rule on/off S
- [ ] G3 Typed filter bar `status = active and revenue > 100` with column/op completion + inline errors (⌘⇧F) M
- [ ] G4 Multi-column sort dialog (per-rule toggle, type-aware labels), Shift+click header adds a rule S
- [ ] G5 Hidden columns saved per table + "Hide column" header item S
- [ ] G6 Find in grid ⌘F: match case, n of m, Enter/Shift+Enter, highlight all + current, scroll M
- [ ] G7 Pager: jump to page, sizes 25–1000, page size remembered per connection; default page size setting S
- [ ] G8 Exact `COUNT(*)` under ~500k estimate, compact "1.2K / 3.4M", "+" when estimated S
- [ ] G9 Large-value truncation (>1000 chars / big JSON / binary placeholders) + full fetch by key for inspector/editor L
- [ ] G10 Enum dropdown; enum-array / MySQL SET chips (ColumnInfo enum values from `pg_enum` / `COLUMN_TYPE`) M
- [ ] G11 Array chip editor; vector editor `[1.0, 2.0]` with validation M
- [ ] G12 Set as NULL / DEFAULT / Empty / `now()`; Backspace on empty → NULL (raw SQL token marker) M
- [ ] G13 Column metadata: has_default / generated / auto_increment → read-only cells, "Auto"/"DEFAULT" placeholders, REQUIRED badge M
- [ ] G14 Enter/F2 edits selected cell; Delete sets NULL S
- [ ] G15 Paste TSV / JSON array as new rows S
- [ ] G16 Refetch only changed rows after commit (PK filter / RETURNING) M
- [ ] G17 Dialect-correct SQL copy (identifier quoting, backslash escaping, `X'..'` binary) via Rust `rows_as_sql` S
- [ ] G18 FK peek popover (key/value, Open in table, Not found) + pinned floating FK panels M
- [ ] G19 User-defined FK remap + inferred FKs (`x_id` → `x`/`xs`.id, Supabase `user_id` → `auth.users`) M
- [ ] G20 Inspector: editable typed fields, NULL toggle, Update/Reset/Delete, validation M
- [ ] G21 Inspector JSON tab editable + Format; copy-row menu (JSON/CSV/SQL/TSV/Markdown) M
- [ ] G22 Inspector SQL tab SELECT/INSERT/UPDATE/DELETE by PK S
- [ ] G23 Multi-row inspector (value per row, `IN (...)`) M
- [ ] G24 Pending changes: group same-row edits into one card; highlighted SQL with Copy + per-statement comment S
- [ ] G25 DDL and Redis-key changes in the pending queue (with destructive confirm) M
- [ ] G26 Shortcuts ⌘R refresh, ⌘⇧F filter, ⌘↵ apply changes (grid scope) S
- [ ] G27 Card view toggle (low) M

## 3. Schema browser, DDL, tags, diagrams  (`TablesPanel.tsx`, `features/diagrams`, `Integration::*_template`)

- [ ] D1 Sidebar: virtualized list, per-table "Refresh columns", sort A-Z / Z-A / Tags S
- [ ] D2 Table tags: per schema+connection, colour presets, CRUD, multi-tag a table, group by tag, persisted collapse, colour dots, drag-reorder M
- [ ] D3 Tree keyboard navigation; drag a table into a split pane; Shift+click opens a new tab S
- [ ] D4 Copy real DDL (`load_ddl`, sketch fallback) S
- [ ] D5 Create-table form: engine type picker + length/precision/scale, PK/Nullable/Unique/Auto-inc/Unsigned, default (NULL/NOW()/empty/custom), FKs with ON DELETE/UPDATE, quoted identifiers, ClickHouse ENGINE/ORDER BY, Mongo = collection M
- [ ] D6 Edit table (ALTER): rename table; add/drop/rename column; type/null/default/unique; add/drop FK; SQLite rebuild L
- [ ] D7 Duplicate table form (structure / +data / +indexes) S
- [ ] D8 Truncate / drop table / view with type-the-name confirm; close affected tabs; refresh S
- [ ] D9 Create / rename / drop database and schema (PG IF NOT EXISTS + owner; notes where engines can't rename) M
- [ ] D10 ERD: persist positions + viewport; snap to grid; highlight selected relation; double-click opens table S
- [ ] D11 ERD: per-table colour + edge bend points (diagram customizations store) M
- [ ] D12 ERD / designer export PNG + SVG to file (whole / selection) S
- [ ] D13 ERD → "Edit in designer" copy S
- [ ] D14 Designer: inspector side panel (schema, colour, column type picker, PK, FK, nullable, default, unique, comment, per-table SQL preview); 4-side handles M
- [ ] D15 Designer relations: composite columns, ON DELETE/UPDATE, line colour/width M
- [ ] D16 Designer sticky notes: colour, font size, resizable, hide on export, show/hide all M
- [ ] D17 Designer auto-layout TB / LR / compact; grid snap sizes; multi-select bulk colour/focus/duplicate/delete S
- [ ] D18 Autosave (debounced) + automatic versions (skip identical, keep 20) + named checkpoints + restore popover M
- [ ] D19 Engine-correct SQL generation in Rust (quoting, schema-qualified, FK ALTERs with actions, warnings) + "Open in query tab" M

## 4. Query editor, results, saved queries, logs  (`features/editor`, `features/queries`, `features/history`)

- [ ] E1 Run scope setting honoured; multi-cursor run S
- [ ] E2 Per-statement batch: status success/error/skipped, stop-on-error toggle, progress, cancel between statements; result tabs show status + time M
- [ ] E3 Results + running state survive tab switches (per-tab store) S
- [ ] E4 "Run without limit" on truncated results; Unlimited max-rows option S
- [ ] E5 DB error position / hint / detail with editor squiggle (`@codemirror/lint`) M
- [ ] E6 Formatter dialect map per family; Mongo JSON format S
- [ ] E7 Find / replace (`@codemirror/search`) S
- [ ] E8 Collapse results pane; auto-show on run S
- [ ] E9 Clause-aware completion; lazy column fetch for mentioned tables S
- [ ] E10 Per-dialect keyword + function catalogs (`src/lib/sqlCatalog.ts`) M
- [ ] E11 `{{var}}` completion (params, dashboard vars incl. `.start/.end`) S
- [ ] E12 Result export/copy: TSV, Markdown, SQL INSERT, NDJSON; selected rows S
- [ ] E13 Tab ↔ saved query link: ⌘S overwrites with diff confirm / Save as new M
- [ ] E14 Saved-query folders (nested, move, inline create), duplicate-name check M
- [ ] E15 Saved-query versions (auto on change, keep 10, popover, restore) M
- [ ] E16 Saved query rename / duplicate / duplicate to another connection / delete confirm; bulk export `.sql` S
- [ ] E17 Mongo shell subset: `db.c.find/aggregate/insertOne/updateMany/deleteOne…`, `show dbs/collections`, `use` → command documents M
- [ ] E18 Log viewer: All/System/User filter, Table/Operation/User columns, open table, server-side paging 25–200 with total S
- [ ] E19 Log retention (delete older than N days) + per-statement logging with type + table S
- [ ] E20 Inline AI ⌘I at the cursor (streamed insert, refine selection, prompt-as-comment) — see AI8
- [ ] E21 Open a `.sql` file into the editor S

## 5. Dashboards  (`features/dashboards`, `model/documents.rs`)

- [ ] DB1 Typed variables: date_range, text, number, select, multi_select, boolean + options editor M
- [ ] DB2 Variable controls: range calendar, input, dropdown, multi-select (all/clear), switch, in a toolbar popover M
- [ ] DB3 Safe per-engine substitution `{{name}}`, `{{name.start/.end}}`, lists, booleans, dates (fixes B3); re-run on change S
- [ ] DB4 Drag / resize / draw-to-create layout with swap compaction L
- [ ] DB5 Tile right-click Edit / Duplicate / Remove S
- [ ] DB6 Metric tile: format Number/Decimal/Currency/Percent, 24 currencies, compare to 2nd value, colour by change, gradient background S
- [ ] DB7 Text tile arithmetic `{{a/b*100}}` (safe evaluator) S
- [ ] DB8 Area tile stacked S
- [ ] DB9 Chart presentation: legend Auto/Always/Hide; value format Number/Compact/Currency/Percent in axes + tooltips S
- [ ] DB10 Tile colours: 14 swatches + gradients in both themes S
- [ ] DB11 Conditions: `is null` / `is not null`, per-rule background, live match highlight S
- [ ] DB12 Tile inspector: column-aware completion, draft Reset / Cancel S
- [ ] DB13 Refresh 5 s / 10 s; `R` refresh, ⌘S save; fullscreen; dashboard title; result cache across tab switches S
- [ ] DB14 Map tile on MapLibre basemap (needs CSP) M
- [ ] DB15 GIF picker (needs CSP / proxy) M
- [ ] DB16 AI-generated dashboard (empty state + table menu) — see AI6

## 6. Workflows  (`features/workflows`, `commands/workflows.rs`, new `services/workflows/`)

- [ ] W1 Graph model (`nodes` / `edges`), legacy step lists migrate to a chain of query nodes L
- [ ] W2 Engine: topological levels, parallel within a level, handle-keyed outputs, merge, branch pruning, loops, cycle detection, cancel token, streamed node status L
- [ ] W3 Template syntax `{{col}}`, arithmetic, `{{rows}}` / `.table/.json/.list/.html`, `{{rows.count}}`, `{{date.now/today/time/unix}}` S
- [ ] W4 Canvas on React Flow: drag, connect, minimap, controls, context-menu delete, connection rules, animated edges M
- [ ] W5 Node picker with search + categories; per-node help; per-node status + 200-row output preview; run one node / run all / stop M
- [ ] W6 Nodes — Query (input templating, preview) M
- [ ] W7 Nodes — HTTP request (methods, templated headers/body, response path → rows, abort) S
- [ ] W8 Nodes — Read file / Save file (CSV, TSV, JSON, NDJSON, Markdown, SQL INSERT) S
- [ ] W9 Nodes — Insert into table (field → column by name) M
- [ ] W10 Nodes — Filter, Row-count gate, Branch true/false, Repeat, Delay, Regex (+ live test) S
- [ ] W11 Nodes — JavaScript (sandboxed, console capture, abort) M
- [ ] W12 Nodes — Desktop notification, Email (SMTP, sealed password), Slack webhook (sealed), Note M
- [ ] W13 Schedule trigger: interval (≥10 s) / at time + weekdays + timezone; "Schedules active" toggle; Settings → Schedules list M
- [ ] W14 Remember last email/Slack config S
- [ ] W15 Destructive statements need explicit per-node opt-in (fixes B4) S
- [ ] W16 AI-generated workflow — see AI7

## 7. Library: folders, versions, duplicate, copy-to-connection  (dashboards, workflows, diagrams, saved queries)

- [ ] L1 Folders for documents (create/rename/delete/move, open state) M
- [ ] L2 Duplicate "(copy)" and Copy to another connection S
- [ ] L3 Delete confirmation, rename from list S
- [ ] L4 "This connection only" list filter S
- [ ] L5 Palette lists documents; open / run workflow from palette S

## 8. Redis, import / export, engine-specific UX  (`features/keys`, `features/transfer`, `lib/export.ts`)

- [ ] R1 Cursor-paged key scan (no 5 000 cap), server search `SCAN MATCH` dropdown M
- [ ] R2 Key tree: separator detection `: / .`, type icons, "show more" paging, value previews toggle M
- [ ] R3 Key metadata: memory usage, encoding, idle time S
- [ ] R4 TTL editor: number + unit, Apply, Remove TTL S
- [ ] R5 String editor in CodeMirror with dirty tracking S
- [ ] R6 Hash / List / Set / Sorted-set editors (add / edit / remove; member + score) through `execute_query` M
- [ ] R7 RedisJSON keys (`JSON.GET`, `JSON.SET`) S
- [ ] R8 Create key dialog (all types + TTL, refuse existing); rename (`RENAMENX`) M
- [ ] R9 Redis console command catalog (≈190 commands, argument hints, descriptions) M
- [ ] X1 Export formats TSV, NDJSON, Markdown, SQL INSERT everywhere (table, result, selection, clipboard) S
- [ ] X2 Export options dialog: columns, delimiter, null text, header, BOM, JSON array/envelope, pretty, preview, formula-injection quoting M
- [ ] X3 Multi-table export as ZIP / one combined JSON (+`__schema`) / one SQL file; include data toggle; progress events M
- [ ] X4 CSV import: delimiter, quote, header toggle, preview, column mapping with Skip M
- [ ] X5 Import truncate-first and on-conflict error / skip / update S
- [ ] X6 JSON import of `{table:[rows]}` + `__schema` as an operation checklist M
- [ ] X7 In-app `.sql` import with dialect-aware streaming splitter + progress L
- [ ] X8 ClickHouse: `query_id` cancel (`KILL QUERY`), `ALTER TABLE … UPDATE/DELETE` row edits, error position S
- [ ] X9 DynamoDB: grid editing via PartiQL, local aggregates over capped scans, "page only" sort hints, retry with backoff M

## 9. Workbench shell, settings, themes, shortcuts  (`features/shell`, `features/settings`, `stores/workspace.ts`, `lib/keymap.ts`)

- [ ] S1 Multiple windows (New Window ⌘⇧N; connection in its own window; settings broadcast) L
- [ ] S2 Window size/position saved S
- [ ] S3 Native menu (About, Check for updates, New window, Close tab/window, zoom, What's new) M
- [ ] S4 Whole-app zoom 0.9–1.5 (⌘+ ⌘- ⌘0) S
- [ ] S5 Update check at launch + update bar; changelog / What's new page; About dialog M
- [ ] S6 Collapsible left sidebar ⌘J (width per connection); global inspector toggle ⌘L M
- [ ] S7 Layout presets Default / Focus / Explore / Inspect / AI M
- [ ] S8 Tabs: drag reorder; preview tab (reuse on single click); unsaved-changes confirm M
- [ ] S9 Panes: up to 4, horizontal + vertical, drop tab on pane edge, drag sidebar item into a pane, Split down / Close pane L
- [ ] S10 Palette: fuzzy match, recents, "Open as new query" for SQL input, hide sections S
- [ ] S11 Editor fonts: System + Inconsolata, Victor Mono, Martian Mono, IBM Plex Mono, Space Mono, Ubuntu Mono S
- [ ] S12 Settings: grid scrollbar size, date-picker toggle S
- [ ] S13 Themes: Ember, Nocturne, Glacier, Midnight, Parchment beside Dark / Light M
- [ ] S14 Custom theme editor (9 token groups, live preview) + JSON import/export (also reads `dbpro-theme` files) L
- [ ] S15 Native window chrome follows the app theme S
- [ ] S16 Shortcut scopes (global / editor / grid) for conflicts; unbind button; new actions: sidebar ⌘J, inspector ⌘L, new query ⌘N, ERD ⌘⇧D, settings ⌘,, refresh ⌘R, prev/next tab ⌘⇧[ ⌘⇧], AI inline ⌘I, zoom S
- [ ] S17 Error log file (rotating) + panic hook + error toast S

## 10. AI  (`features/chat`, `services/agent`, `services/ai.rs`)

- [ ] AI1 Saved chats per connection (store), history popover (title, relative time, switch, delete, new), auto-title L
- [ ] AI2 Several providers at once with their own keys; Google Gemini; model list fetch + cache + refresh; Ollama test; model picker in chat M
- [ ] AI3 Data-access mode Off / Schema only / Schema & data (schema-only drops row tools) S
- [ ] AI4 `@` mentions for tables, saved queries, dashboards M
- [ ] AI5 Code blocks run inline (SQL / Redis / Mongo) with a result grid; Copy / export results as CSV / JSON / SQL M
- [ ] AI6 Create / update a dashboard from chat; empty-dashboard prompt; "Generate dashboard" on a table M
- [ ] AI7 Create a workflow from chat; empty-workflow prompt M
- [ ] AI8 Inline AI in the editor ⌘I (cursor insert, refine selection, comment option); "Ask AI about this SQL" M
- [ ] AI9 Create a diagram from chat; empty-diagram prompt; open `render_graph` in the designer M
- [ ] AI10 Dynamic suggestions from real tables; "Ask AI" in grid row menu; first-run AI dialog S

## 11. Engine robustness  (`guard/`, `services/query.rs`, `state.rs`, adapters)

- [ ] N1 Session per tab (SET / temp tables / BEGIN persist), dead-session detection + reconnect, closed with the tab L
- [ ] N2 `RETURNING` for PG family + SQLite ≥ 3.35 S
- [ ] N3 PgBouncer / transaction poolers: disable statement cache for `pgbouncer=true`, port 6543, `pooler` hosts S
- [ ] N4 MSSQL reconnect on I/O error S
- [ ] N5 Persisted schema cache in the app store (dedupe in-flight, force refresh) M
- [ ] N6 File-based master key when no OS keychain (0600, warned) S
