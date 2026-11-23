// SOT: ui-preview-bridge, tauri-stub, browser-preview-fixtures
//
// WHAT:  A stand-in for Tauri's IPC bridge, so the webview can be opened and
//        clicked through in an ordinary browser with no Rust core behind it.
// WHY:   Building the desktop shell needs the platform's webkit2gtk headers.
//        Reviewing a UI change should not.
// HOW:   Injected into index.html by the `db-free:ui-preview` plugin in
//        vite.config.ts, and only ever when DBFREE_PREVIEW=1 on a dev server —
//        it is not reachable from `vite build`.
// WHERE: vite.config.ts, package.json (`pnpm preview:ui`)
(function () {
  var settings = {
    theme: "dark", accent: "blue", uiFont: "jetbrains-mono", editorFont: "jetbrains-mono",
    uiFontSize: 13, editorFontSize: 13, gridDensity: "cozy",
    alternatingRows: true, rememberTableState: true, columnPreview: true,
    maxQueryRows: 5000, nullDisplay: "NULL", showResultsPane: true,
    condenseSqlWhenFormatting: false, runScope: "all", executionMode: "review",
    commandMenuSections: ["create", "navigation", "connections", "tables", "saved_queries", "settings"],
    inspectorTabs: ["columns", "indexes", "keys"],
    confirmDestructive: true, crashReportsOptIn: false,
    ai: { provider: "openai", model: "gpt-4o-mini", baseUrl: null, hasApiKey: false, autonomy: "ask_on_write", dataAccess: "schema", defaultProfile: null, profiles: [], maxSteps: 12 },
    keybindings: [],
    openConnectionsInNewWindow: false, uiZoom: 1, reuseTablePreviewTab: false, gridScrollbarSize: "standard", datePickerEnabled: true,
    nativeToolPaths: {}, showPinned: true, showRecents: true, defaultPageSize: 100,
    logRetentionDays: 30, inlineAiPromptAsComment: false,
    commandMenuHiddenSections: [], customThemes: [], checkUpdatesOnLaunch: true,
    snippets: [{ prefix: "audit", name: "Audit columns", body: "created_at, updated_at, created_by", language: "sql" }],
  };

  function conn(id, name, engine, environment, host, port, database) {
    return {
      id: id, name: name, engine: engine, environment: environment,
      readOnly: environment === "production", host: host, port: port,
      database: database, username: "app", filePath: null, sslMode: "prefer",
      ssh: { enabled: false, host: null, port: 22, user: null, auth: "password", keyPath: null, hostKey: null }, hasSshSecret: false, folder: null, color: null, favorite: false,
      hasSecret: true, createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z",
      extra: { uriParams: null, srv: false, tls: { ca: null, cert: null, key: null }, tagIds: [], folderIds: [], transient: false, tunnelOriginHost: null },
      auth: { method: "password" },
      hasAuthSecrets: { awsSecretAccessKey: false, awsSessionToken: false, azureClientSecret: false, azurePassword: false, azureAccessToken: false, gcpServiceAccountJson: false },
    };
  }

  var connections = [
    conn("c1", "Local Postgres", "postgres", "local", "localhost", 5432, "app_dev"),
    conn("c2", "Staging MySQL", "mysql", "staging", "db.staging.internal", 3306, "shop"),
    conn("c3", "Prod Postgres", "postgres", "production", "db.prod.internal", 5432, "shop"),
    conn("c4", "Cache Redis", "redis", "local", "localhost", 6379, "0"),
    conn("c5", "Events MongoDB", "mongodb", "staging", "mongo.staging.internal", 27017, "events"),
  ];

  function tbl(name, rows, kind) {
    return { schema: "public", name: name, kind: kind || "table", rowEstimate: rows };
  }

  var catalog = {
    schemas: [
      { name: "public", tables: [
        tbl("customers", 1842), tbl("orders", 9310), tbl("order_items", 27655),
        tbl("products", 412), tbl("payments", 8801), tbl("shipments", 7233),
        tbl("active_customers", 1203, "view"),
      ] },
      { name: "analytics", tables: [tbl("daily_revenue", 730), tbl("cohorts", 96)] },
    ],
  };

  var columns = [
    { name: "id", dataType: "int4", nullable: false, primaryKey: true, ordinal: 0 },
    { name: "email", dataType: "text", nullable: false, primaryKey: false, ordinal: 1 },
    { name: "full_name", dataType: "text", nullable: true, primaryKey: false, ordinal: 2 },
    { name: "signed_up_at", dataType: "timestamp", nullable: false, primaryKey: false, ordinal: 3 },
    { name: "lifetime_value", dataType: "numeric", nullable: true, primaryKey: false, ordinal: 4 },
    { name: "is_active", dataType: "bool", nullable: false, primaryKey: false, ordinal: 5 },
    { name: "preferences", dataType: "jsonb", nullable: true, primaryKey: false, ordinal: 6 },
  ];

  var names = ["Ada Lovelace", "Grace Hopper", "Alan Turing", "Katherine Johnson",
               "Edsger Dijkstra", "Barbara Liskov", "Ken Thompson", "Margaret Hamilton"];
  var rows = [];
  for (var i = 0; i < 240; i++) {
    rows.push([
      { t: "int", v: i + 1 },
      { t: "text", v: "user" + (i + 1) + "@example.com" },
      i % 7 === 3 ? { t: "null" } : { t: "text", v: names[i % names.length] },
      { t: "date_time", v: "2026-0" + ((i % 9) + 1) + "-1" + (i % 9) + " 09:" + String(i % 60).padStart(2, "0") + ":00" },
      { t: "decimal", v: (i * 37.5 + 12).toFixed(2) },
      { t: "bool", v: i % 3 !== 0 },
      i % 5 === 0 ? { t: "null" } : { t: "json", v: { theme: i % 2 ? "dark" : "light", newsletter: i % 3 === 0, tags: ["a", "b"] } },
    ]);
  }


  // Foreign keys between the public tables: the ERD, FK peek and the designer.
  function fk(name, from, fromCols, to, toCols) {
    return { name: name, fromSchema: "public", fromTable: from, fromColumns: fromCols, toSchema: "public", toTable: to, toColumns: toCols };
  }
  var foreignKeys = [
    fk("orders_customer_fk", "orders", ["customer_id"], "customers", ["id"]),
    fk("items_order_fk", "order_items", ["order_id"], "orders", ["id"]),
    fk("items_product_fk", "order_items", ["product_id"], "products", ["id"]),
    fk("payments_order_fk", "payments", ["order_id"], "orders", ["id"]),
    fk("shipments_order_fk", "shipments", ["order_id"], "orders", ["id"]),
  ];

  // One saved document of each kind, so the library, dashboards, the designer
  // and workflows open on content. Saves are kept for the session.
  var stamp = "2026-11-20T09:00:00Z";
  function widget(id, title, kind, sql, x, y, w, h, extra) {
    var base = {
      id: id, title: title, kind: kind, sql: sql, x: x, y: y, w: w, h: h, tint: "series-1",
      showChange: false, maxValue: null, text: null, url: null, xLabel: null, yLabel: null,
      horizontal: false, showPercent: true, showValues: false, pulse: false, conditions: [],
      format: "number", currency: "USD", colorByChange: false, bg: null, stacked: true,
      legendMode: "auto", valueFormat: "number",
    };
    return Object.assign(base, extra || {});
  }
  function dcol(name, dataType, pk) {
    return { name: name, dataType: dataType, primaryKey: !!pk, nullable: !pk, unique: false, autoIncrement: !!pk, defaultValue: null, comment: null };
  }
  function dtable(id, name, x, y, cols) {
    return { id: id, name: name, x: x, y: y, columns: cols, schema: "public", color: null, comment: null };
  }
  function drel(id, from, fromCol, to, toCol) {
    return { id: id, fromTable: from, fromColumn: fromCol, toTable: to, toColumn: toCol, extraColumns: [], name: null, onDelete: null, onUpdate: null, color: null, width: null, fromSide: null, toSide: null, waypoints: [] };
  }
  var documents = [
    { id: "d1", kind: "dashboard", connectionId: "c1", name: "Sales overview", tags: [], createdAt: stamp, updatedAt: stamp,
      body: { kind: "dashboard", data: { title: "", refreshSeconds: 0, layoutVersion: 1, variables: [],
        widgets: [
          widget("w1", "Customers", "metric", "select count(*) from customers", 0, 0, 3, 3),
          widget("w2", "Lifetime value", "metric", "select sum(lifetime_value) from customers", 3, 0, 3, 3, { format: "currency" }),
          widget("w3", "Active customers", "progress", "select count(*) from active_customers", 6, 0, 6, 3, { maxValue: 2500 }),
          widget("w4", "Value by customer", "bar", "select full_name, lifetime_value from customers limit 12", 0, 3, 6, 6),
          widget("w5", "Recent customers", "table", "select id, email, full_name from customers limit 20", 6, 3, 6, 6),
        ] } } },
    { id: "d2", kind: "diagram", connectionId: "c1", name: "Orders model", tags: [], createdAt: stamp, updatedAt: stamp,
      body: { kind: "diagram", data: { gridSize: 16, notesVisible: true, notes: [],
        tables: [
          dtable("t1", "customers", 40, 40, [dcol("id", "int4", true), dcol("email", "text"), dcol("full_name", "text")]),
          dtable("t2", "orders", 420, 40, [dcol("id", "int4", true), dcol("customer_id", "int4"), dcol("total", "numeric")]),
          dtable("t3", "order_items", 800, 40, [dcol("id", "int4", true), dcol("order_id", "int4"), dcol("product_id", "int4")]),
          dtable("t4", "products", 800, 300, [dcol("id", "int4", true), dcol("name", "text"), dcol("price", "numeric")]),
        ],
        relations: [drel("r1", "t2", "customer_id", "t1", "id"), drel("r2", "t3", "order_id", "t2", "id"), drel("r3", "t3", "product_id", "t4", "id")],
      } } },
    { id: "d3", kind: "workflow", connectionId: "c1", name: "Nightly order check", tags: [], createdAt: stamp, updatedAt: stamp,
      body: { kind: "workflow", data: { schedulesActive: false, steps: [],
        nodes: [
          { id: "n1", x: 80, y: 120, label: "", continueOnError: false, config: { kind: "schedule", mode: "at_time", intervalAmount: 1, intervalUnit: "days", time: "02:00", days: ["mon", "tue", "wed", "thu", "fri"], timezone: "" } },
          { id: "n2", x: 380, y: 120, label: "Open orders", continueOnError: false, config: { kind: "query", connectionId: null, sql: "select count(*) from orders where shipped_at is null", allowDestructive: false } },
          { id: "n3", x: 680, y: 120, label: "", continueOnError: false, config: { kind: "notification", title: "Open orders", body: "{{n2.rows.0.count}} orders waiting" } },
        ],
        edges: [{ id: "e1", source: "n1", target: "n2", sourceHandle: null }, { id: "e2", source: "n2", target: "n3", sourceHandle: null }],
      } } },
  ];

  // Server admin: overview stats and a few rows per admin list.
  var serverStats = {
    collectedAt: "2026-11-23T12:00:00Z",
    groups: [
      { title: "Activity", stats: [
        { label: "Connections", value: "14", unit: null, hint: "of 100 allowed", numeric: 14 },
        { label: "Active queries", value: "3", unit: null, hint: null, numeric: 3 },
        { label: "Transactions", value: "1,284", unit: "/s", hint: null, numeric: 1284 },
      ] },
      { title: "Storage", stats: [
        { label: "Database size", value: "2.4", unit: "GB", hint: null, numeric: 2.4 },
        { label: "Cache hit ratio", value: "99.2", unit: "%", hint: null, numeric: 99.2 },
        { label: "Dead tuples", value: "18,402", unit: null, hint: null, numeric: 18402 },
      ] },
    ],
  };
  function obj(kind, name, detail, badge) { return { reference: { kind: kind, name: name, parent: kind === "session" || kind === "lock" || kind === "user" || kind === "role" ? null : "public" }, detail: detail, badge: badge }; }
  var objects = {
    view: [obj("view", "active_customers", "1,203 rows", null)],
    function: [obj("function", "order_total(order_id int4)", "returns numeric", "plpgsql")],
    trigger: [obj("trigger", "orders_touch", "orders · before update", null)],
    index: [obj("index", "orders_customer_idx", "orders (customer_id)", "btree"), obj("index", "customers_email_key", "customers (email)", "unique")],
    sequence: [obj("sequence", "orders_id_seq", "last 9310", null)],
    session: [obj("session", "4182 app", "select * from orders …", "active"), obj("session", "4190 app", "idle 12s", "idle"), obj("session", "4201 report", "idle in transaction 3m", "idle")],
    lock: [obj("lock", "orders · RowExclusiveLock", "pid 4182", "granted")],
    user: [obj("user", "app", "login", null), obj("user", "report", "login", null)],
    role: [obj("role", "readonly", "3 members", null)],
  };

  var history = [
    { id: 3, connectionId: "c1", sql: "select * from customers order by signed_up_at desc limit 100", status: "ok", origin: "user", error: null, elapsedMs: 18, rowCount: 100, executedAt: "2026-11-23T11:58:00Z", statementType: "SELECT", tableName: "customers", operation: "read", user: "app", fromEditor: true },
    { id: 2, connectionId: "c1", sql: "update orders set status = 'shipped' where id = 9310", status: "ok", origin: "user", error: null, elapsedMs: 6, rowCount: 1, executedAt: "2026-11-23T11:52:00Z", statementType: "UPDATE", tableName: "orders", operation: "write", user: "app", fromEditor: true },
    { id: 1, connectionId: "c1", sql: "select count(*) from shipmnts", status: "error", origin: "user", error: "relation \"shipmnts\" does not exist", elapsedMs: 2, rowCount: null, executedAt: "2026-11-23T11:50:00Z", statementType: "SELECT", tableName: null, operation: "read", user: "app", fromEditor: true },
  ];

  // Redis keys for the key browser (Cache Redis).
  var redisKeys = [
    { name: "session:4f2a", kind: "hash", preview: null },
    { name: "session:9c1d", kind: "hash", preview: null },
    { name: "cart:1842", kind: "list", preview: null },
    { name: "leaderboard:weekly", kind: "zset", preview: null },
    { name: "feature:flags", kind: "set", preview: null },
    { name: "config:theme", kind: "string", preview: "dark" },
    { name: "events:orders", kind: "stream", preview: null },
  ];

  // Connected ids, so a window-focus resync keeps what the preview connected.
  var sessions = [];

  var caps = { sql: true, namespaces: true, fixedColumns: true, paging: true,
               rowEstimate: true, views: true, transactions: true,
               exactEstimate: false, describesFields: true };

  var responses = {
    list_connections: connections,
    get_settings: settings,
    list_saved_queries: [
      { id: "q1", connectionId: "c1", name: "Recent orders", sql: "select *\nfrom orders\norder by created_at desc\nlimit 50", tags: ["ops"], createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z" },
      { id: "q2", connectionId: "c1", name: "Top customers", sql: "select full_name, lifetime_value\nfrom customers\norder by lifetime_value desc\nlimit 20", tags: [], createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z" },
    ],
    list_buffers: [],
    list_history: [],
    load_foreign_keys: foreignKeys,
    server_stats: serverStats,
    detect_native_tools: {
      method: "pg_dump", formats: ["custom", "plain", "directory", "tar"], restoreNeedsDisconnect: false, backupNeedsDisconnect: false, note: null,
      tools: [
        { tool: "pg_dump", path: "/opt/homebrew/bin/pg_dump", version: "pg_dump (PostgreSQL) 16.2", overridden: false, package: "postgresql" },
        { tool: "pg_restore", path: "/opt/homebrew/bin/pg_restore", version: "pg_restore (PostgreSQL) 16.2", overridden: false, package: "postgresql" },
        { tool: "psql", path: "/opt/homebrew/bin/psql", version: "psql (PostgreSQL) 16.2", overridden: false, package: "postgresql" },
      ],
    },
    list_library: { folders: [], placements: [] },
    list_diagram_customizations: [],
    list_document_versions: [],
    list_workflow_schedules: [],
    ai_chat_list: [],
    check_update: { current: "0.7.0", available: null, notes: null, published: null },
    download_update: { current: "0.7.0", available: "0.8.0", notes: "shadcn/ui migration", published: "2026-09-09T00:00:00Z" },
    credential_status: { storage: "keychain", keyPath: null, lost: false, lostSecrets: 0 },
    list_connection_org: { tags: [], folders: [] },
    list_table_tags: { tags: [], links: [], untaggedCollapsed: [] },
    ping_session: 4,
    list_pins: [],
    list_recents: [],
    load_catalog: catalog,
    load_columns: columns,
    load_ddl: "create table public.orders (\n  id int4 primary key,\n  customer_id int4 not null,\n  total numeric(10,2)\n);",
  };

  function clone(v) { return JSON.parse(JSON.stringify(v)); }

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, payload) {
      var req = payload && payload.req;
      if (cmd === "fetch_table_page") {
        var offset = (req && req.offset) || 0;
        var limit = (req && req.limit) || 50;
        return Promise.resolve({
          columns: clone(columns),
          rows: clone(rows).slice(offset, offset + limit),
          offset: offset, total: rows.length, totalExact: true,
          columnTraits: [], truncated: [], editable: true,
        });
      }
      if (cmd === "execute_query") {
        // Aggregates (dashboard metrics) get one number; anything else the customer rows.
        var single = /^\s*select\s+(count|sum)\(/i.exec((req && req.sql) || "");
        if (single) {
          return Promise.resolve({
            statements: [{ kind: "rows", result: { columns: [{ name: single[1], typeName: "numeric" }], rows: [[{ t: "decimal", v: single[1].toLowerCase() === "count" ? "1842" : "48210.50" }]], truncated: false } }],
            totalRows: 1, elapsedMs: 4,
          });
        }
        return Promise.resolve({
          statements: [{ kind: "rows", result: {
            columns: columns.map(function (c) { return { name: c.name, typeName: c.dataType }; }),
            rows: clone(rows).slice(0, 25), truncated: false,
          } }],
          totalRows: 25, elapsedMs: 12,
        });
      }
      if (cmd === "save_settings") {
        // The store reads the saved settings back, so a change (theme, density…) sticks.
        settings = clone(req.settings);
        responses.get_settings = settings;
        return Promise.resolve(clone(settings));
      }
      if (cmd === "active_sessions") return Promise.resolve(sessions.slice());
      if (cmd === "list_documents") {
        return Promise.resolve(clone(documents.filter(function (d) { return !req || d.kind === req.kind; })));
      }
      if (cmd === "save_document" || cmd === "autosave_document") {
        var doc = clone(req.document);
        if (!doc.id) doc.id = "d" + (documents.length + 1);
        doc.createdAt = doc.createdAt || stamp;
        doc.updatedAt = new Date().toISOString();
        documents = documents.filter(function (d) { return d.id !== doc.id; }).concat([doc]);
        return Promise.resolve(clone(doc));
      }
      if (cmd === "list_objects") return Promise.resolve(clone(objects[req && req.kind] || []));
      if (cmd === "list_history_page") {
        var page = history.filter(function (h) { return !req || !req.origin || h.origin === req.origin; });
        var from = (req && req.offset) || 0;
        return Promise.resolve({ entries: clone(page).slice(from, from + ((req && req.limit) || 50)), total: page.length });
      }
      if (cmd === "describe_session") {
        // Each connection answers as its own engine; Redis has the key browser.
        var target = connections.find(function (c) { return c.id === (req && req.connectionId); }) || connections[0];
        if (target.engine === "redis") {
          return Promise.resolve({
            engine: "redis", serverVersion: "Redis 7.2.4", database: "0", databases: ["0", "1"], manualTransactions: false,
            capabilities: { sql: false, namespaces: false, fixedColumns: false, paging: true, rowEstimate: true, views: false, transactions: false, exactEstimate: true, describesFields: true },
            objectKinds: ["database", "stream", "session", "slow_query"], tools: ["stats", "key_browser", "pub_sub"],
          });
        }
        return Promise.resolve({
          engine: target.engine, capabilities: caps, manualTransactions: true,
          objectKinds: ["schema", "table", "view", "index", "function", "trigger", "sequence", "session", "lock", "user", "role"],
          tools: ["stats", "erd", "vector_search"], serverVersion: target.engine === "mysql" ? "MySQL 8.4" : "PostgreSQL 16.2",
          database: target.database, databases: [target.database, "postgres"],
        });
      }
      if (cmd === "scan_keys") {
        return Promise.resolve({ cursor: "0", done: true, total: redisKeys.length, keys: clone(redisKeys) });
      }
      if (cmd === "key_info") {
        var hit = redisKeys.find(function (k) { return k.name === (req && req.key); }) || redisKeys[0];
        return Promise.resolve({ kind: hit.kind, typeName: hit.kind, ttlMs: hit.name.indexOf("session:") === 0 ? 3540000 : -1, memoryBytes: 512, encoding: "listpack", idleSeconds: 12, length: 4 });
      }
      if (cmd === "execute_batch") {
        // One statement, answered with the customer rows through the progress channel.
        var sql = (req && req.sql) || "";
        var onEvent = payload.onEvent && payload.onEvent.onmessage;
        var outcome = {
          statements: [{ kind: "rows", result: {
            columns: columns.map(function (c) { return { name: c.name, typeName: c.dataType }; }),
            rows: clone(rows).slice(0, 25), truncated: false,
          } }],
          totalRows: 25, elapsedMs: 12,
        };
        var summary = { total: 1, succeeded: 1, failed: 0, skipped: 0, cancelled: 0, elapsedMs: 12 };
        if (typeof onEvent === "function") {
          onEvent({ type: "planned", statements: [{ index: 0, start: 0, end: sql.length, sql: sql, intent: "read" }] });
          onEvent({ type: "started", index: 0 });
          onEvent({ type: "finished", result: { index: 0, status: "success", elapsedMs: 12, outcome: outcome, error: null } });
          onEvent({ type: "done", summary: summary });
        }
        return Promise.resolve(summary);
      }
      if (cmd === "save_buffer") return Promise.resolve(clone(req.buffer));
      if (cmd === "connect") {
        var opened = connections.find(function (c) { return c.id === (req && req.id); }) || connections[0];
        if (sessions.indexOf(opened.id) < 0) sessions.push(opened.id);
        return Promise.resolve(clone(opened));
      }
      if (cmd === "disconnect") {
        sessions = sessions.filter(function (id) { return id !== (req && req.id); });
        return Promise.resolve(null);
      }
      // Tauri plugins (event listeners, window, menu, dialog…) have no backend
      // here: answer with nothing so listeners register as no-ops.
      if (cmd.indexOf("plugin:") === 0) return Promise.resolve(null);
      if (Object.prototype.hasOwnProperty.call(responses, cmd)) {
        return Promise.resolve(clone(responses[cmd]));
      }
      return Promise.reject({
        kind: "internal",
        message: 'UI preview: "' + cmd + '" needs the Rust core. Run `pnpm tauri dev` for the real thing.',
      });
    },
    transformCallback: function (cb) { var id = Math.random(); window["_cb" + id] = cb; return id; },
    unregisterCallback: function () {},
    convertFileSrc: function (p) { return p; },
    // `getCurrentWindow()` / `getCurrentWebview()` read their labels from here.
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };

  console.info("[db-free] UI preview: the Rust core is stubbed. Data here is fixtures.");
})();
