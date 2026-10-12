// SOT: connection-copy-url, connection-copy-details, password-free-url
import type { ConnectionSummary } from "@/lib/bindings";
import { engineMeta } from "@/lib/engines";
import { encodeUriPart } from "@/lib/connectionUri";

// WHAT:  A connection as a URL a person can paste into chat or a ticket: scheme,
//        user, host, port, database and the options it kept, never the password.
// WHY:   The password is sealed in Rust and never reaches the UI, so a copied URL
//        cannot leak it; "Copy URL" is safe to share, and the recipient pastes it
//        into the New Connection box and adds their own password.
// HOW:   The first scheme the engine registers, except that PostgreSQL uses
//        `postgresql`. TLS is added as the parameter its family spells it with.
// WHERE: src/features/connections/ConnectionsPage.tsx (card menu)
export function connectionUrl(c: ConnectionSummary): string | null {
  const meta = engineMeta(c.engine);
  const scheme = meta.schemes.includes("postgresql") ? "postgresql" : c.extra.srv ? "mongodb+srv" : (meta.schemes[0] ?? c.engine);
  if (meta.form === "file") {
    const path = c.filePath ?? "";
    return path.length > 0 ? `${scheme}://${path}` : null;
  }
  const host = c.host ?? "";
  if (host.length === 0) return null;
  if (meta.form === "http_token" || meta.form === "aws" || meta.form === "gcp") {
    // These forms hold a service address or a cloud region, not a socket: copy what is there.
    return host.includes("://") ? host : `${scheme}://${host}${c.port !== null ? `:${c.port}` : ""}`;
  }
  const user = c.username !== null && c.username.length > 0 ? `${encodeUriPart(c.username)}@` : "";
  const port = c.port !== null && !c.extra.srv ? `:${c.port}` : "";
  const database = c.database !== null && c.database.length > 0 ? `/${encodeUriPart(c.database)}` : "/";
  const params: string[] = c.extra.uriParams !== null ? c.extra.uriParams.split("&").filter((p) => p.length > 0) : [];
  if (c.sslMode !== "prefer" && !params.some((p) => /^(sslmode|ssl|tls)=/i.test(p))) {
    params.push(c.engine === "mongodb" ? `tls=${c.sslMode === "disable" ? "false" : "true"}` : `sslmode=${c.sslMode.replace("_", "-")}`);
  }
  return `${scheme}://${user}${host}${port}${database}${params.length > 0 ? `?${params.join("&")}` : ""}`;
}

// WHAT:  The same connection as semicolon-separated key / value pairs, the shape
//        .NET and ODBC connection strings use: `Host=…;Port=…;Database=…;User ID=…`.
// WHY:   Pasting into a config file or an ODBC data source wants this form, not a URL.
export function connectionDetails(c: ConnectionSummary): string {
  const meta = engineMeta(c.engine);
  if (meta.form === "file") return `Data Source=${c.filePath ?? ""}`;
  const pairs: [string, string | null][] = [
    ["Host", c.host],
    ["Port", c.port !== null ? String(c.port) : null],
    ["Database", c.database],
    ["User ID", c.username],
  ];
  return pairs
    .filter(([, value]) => value !== null && value.length > 0)
    .map(([key, value]) => `${key}=${value ?? ""}`)
    .join(";");
}
