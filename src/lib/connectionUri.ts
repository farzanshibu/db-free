// SOT: connection-uri-split, uri-parts, percent-decoding, uri-secret-params

// WHAT:  A connection string split into its parts: scheme, credentials, host(s),
//        port, database and the query parameters, with nothing resolved.
// WHY:   `new URL()` rejects what database URIs legitimately contain: MongoDB
//        host lists (`h1:27017,h2:27017`), custom schemes it treats as opaque, and
//        passwords with unescaped characters. A small splitter keeps one reading
//        of a connection string for the form box and the picker, and mirrors
//        `parse_uri` in src-tauri/src/services/connection_formats.rs, which reads
//        the same strings out of imported files.
// HOW:   Pure and total: a string that is not `scheme://…` returns null; every
//        other input yields parts, however odd. Parameters stay raw (still
//        percent-encoded) so they can be handed back to a driver unchanged.
// WHERE: src/lib/engines.ts (parseConnectionString), src/features/connections/ConnectionStringBox.tsx

export interface UriParts {
  scheme: string;
  host: string | null;
  port: number | null;
  database: string | null;
  username: string | null;
  password: string | null;
  /// `key=value` pairs in the order written, raw.
  params: string[];
}

/// Parameters that would put a secret into plain-text storage. Mirrors
/// `SECRET_URI_PARAMS` in src-tauri/src/model/connection_org.rs.
const SECRET_PARAMS: readonly string[] = ["password", "passwd", "pwd", "sslpassword", "tlscertificatekeyfilepassword", "secret", "token", "sas_token"];

export function isSecretParam(key: string): boolean {
  return SECRET_PARAMS.includes(key.toLowerCase());
}

// WHAT:  `decodeURIComponent` that never throws: a malformed escape is left as typed.
export function decodeUriPart(raw: string): string {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

export function encodeUriPart(raw: string): string {
  return encodeURIComponent(raw);
}

function parsePort(raw: string): number | null {
  if (!/^\d{1,5}$/.test(raw)) return null;
  const port = Number(raw);
  return port > 0 && port <= 65535 ? port : null;
}

export function splitUri(raw: string): UriParts | null {
  const text = raw.trim();
  const schemeEnd = text.indexOf("://");
  if (schemeEnd <= 0) return null;
  const scheme = text.slice(0, schemeEnd).toLowerCase();
  if (!/^[a-z][a-z0-9+.-]*$/.test(scheme)) return null;
  let rest = text.slice(schemeEnd + 3);
  const hash = rest.indexOf("#");
  if (hash >= 0) rest = rest.slice(0, hash);
  const queryStart = rest.indexOf("?");
  const query = queryStart >= 0 ? rest.slice(queryStart + 1) : "";
  const beforeQuery = queryStart >= 0 ? rest.slice(0, queryStart) : rest;
  const slash = beforeQuery.indexOf("/");
  const authority = slash >= 0 ? beforeQuery.slice(0, slash) : beforeQuery;
  const path = slash >= 0 ? beforeQuery.slice(slash + 1) : "";

  const at = authority.lastIndexOf("@");
  const userinfo = at >= 0 ? authority.slice(0, at) : null;
  const hostport = at >= 0 ? authority.slice(at + 1) : authority;

  let username: string | null = null;
  let password: string | null = null;
  if (userinfo !== null) {
    const colon = userinfo.indexOf(":");
    const user = decodeUriPart(colon >= 0 ? userinfo.slice(0, colon) : userinfo);
    const pass = colon >= 0 ? decodeUriPart(userinfo.slice(colon + 1)) : "";
    username = user.length > 0 ? user : null;
    password = pass.length > 0 ? pass : null;
  }

  let host: string;
  let port: number | null = null;
  if (hostport.includes(",")) {
    host = hostport;
  } else if (hostport.startsWith("[")) {
    const close = hostport.indexOf("]");
    if (close < 0) return null;
    host = hostport.slice(0, close + 1);
    const tail = hostport.slice(close + 1);
    port = tail.startsWith(":") ? parsePort(tail.slice(1)) : null;
  } else {
    const colon = hostport.lastIndexOf(":");
    const maybePort = colon >= 0 ? parsePort(hostport.slice(colon + 1)) : null;
    if (colon >= 0 && maybePort !== null) {
      host = hostport.slice(0, colon);
      port = maybePort;
    } else {
      host = hostport;
    }
  }

  const firstSegment = path.split("/")[0] ?? "";
  const database = decodeUriPart(firstSegment);
  return {
    scheme,
    host: host.length > 0 ? host : null,
    port,
    database: database.length > 0 ? database : null,
    username,
    password,
    params: query.split("&").filter((pair) => pair.length > 0),
  };
}

// WHAT:  The value of one parameter, decoded; case-insensitive on the key, last one wins.
export function paramValue(params: readonly string[], key: string): string | null {
  let found: string | null = null;
  for (const pair of params) {
    const eq = pair.indexOf("=");
    const name = eq >= 0 ? pair.slice(0, eq) : pair;
    if (name.toLowerCase() === key.toLowerCase()) found = eq >= 0 ? decodeUriPart(pair.slice(eq + 1)) : "";
  }
  return found;
}

// WHAT:  Replaces or sets one parameter in a raw `a=1&b=2` string; an empty value removes it.
export function withParam(raw: string | null, key: string, value: string): string | null {
  const kept = (raw ?? "")
    .split("&")
    .filter((pair) => pair.length > 0)
    .filter((pair) => (pair.includes("=") ? pair.slice(0, pair.indexOf("=")) : pair).toLowerCase() !== key.toLowerCase());
  if (value.length > 0) kept.push(`${key}=${encodeUriPart(value)}`);
  return kept.length > 0 ? kept.join("&") : null;
}
