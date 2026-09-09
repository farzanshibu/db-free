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
    accent: "blue", uiFont: "jetbrains-mono", editorFont: "jetbrains-mono",
    uiFontSize: 13, editorFontSize: 13, gridDensity: "cozy",
    alternatingRows: true, rememberTableState: true, columnPreview: true,
    maxQueryRows: 5000, nullDisplay: "NULL", showResultsPane: true,
    condenseSqlWhenFormatting: false, runScope: "all", executionMode: "review",
    commandMenuSections: ["create", "navigation", "connections", "tables", "saved_queries", "settings"],
    inspectorTabs: ["columns", "indexes", "keys"],
    confirmDestructive: true, crashReportsOptIn: false,
    ai: { provider: "openai", model: "gpt-4o-mini", baseUrl: null, hasApiKey: false, autonomy: "ask" },
  };

  function conn(id, name, engine, environment, host, port, database) {
    return {
      id: id, name: name, engine: engine, environment: environment,
      readOnly: environment === "production", host: host, port: port,
      database: database, username: "app", filePath: null, sslMode: "prefer",
      hasSecret: true, createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z",
    };
  }

  var connections = [
    conn("c1", "Local Postgres", "postgres", "local", "localhost", 5432, "app_dev"),
    conn("c2", "Staging MySQL", "mysql", "staging", "db.staging.internal", 3306, "shop"),
    conn("c3", "Prod Postgres", "postgres", "production", "db.prod.internal", 5432, "shop"),
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

  var caps = { sql: true, namespaces: true, fixedColumns: true, paging: true,
               rowEstimate: true, views: true, transactions: true,
               exactEstimate: false, describesFields: true };

  var responses = {
    list_connections: connections,
    active_sessions: [],
    get_settings: settings,
    save_settings: null,
    list_saved_queries: [
      { id: "q1", connectionId: "c1", name: "Recent orders", sql: "select *\nfrom orders\norder by created_at desc\nlimit 50", tags: ["ops"], createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z" },
      { id: "q2", connectionId: "c1", name: "Top customers", sql: "select full_name, lifetime_value\nfrom customers\norder by lifetime_value desc\nlimit 20", tags: [], createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-01-01T00:00:00Z" },
    ],
    list_buffers: [],
    list_history: [],
    list_documents: [],
    load_foreign_keys: [],
    list_objects: [],
    check_update: { current: "0.7.0", available: null, notes: null, published: null },
    download_update: { current: "0.7.0", available: "0.8.0", notes: "shadcn/ui migration", published: "2026-09-09T00:00:00Z" },
    connect: true,
    disconnect: null,
    describe_session: {
      engine: "postgres", capabilities: caps,
      objectKinds: ["schema", "table", "view", "index", "function", "trigger", "sequence"],
      tools: ["stats", "erd"], serverVersion: "PostgreSQL 16.2",
      database: "app_dev", databases: ["app_dev", "app_test", "postgres"],
    },
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
        });
      }
      if (cmd === "execute_query") {
        return Promise.resolve({
          statements: [{ kind: "rows", result: {
            columns: columns.map(function (c) { return { name: c.name, typeName: c.dataType }; }),
            rows: clone(rows).slice(0, 25), truncated: false,
          } }],
          totalRows: 25, elapsedMs: 12,
        });
      }
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
  };

  console.info("[db-free] UI preview: the Rust core is stubbed. Data here is fixtures.");
})();
