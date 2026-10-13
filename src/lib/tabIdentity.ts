// SOT: tab-identity, connection-identity-colour, tab-connection-tooltip
import type { ConnectionColor, ConnectionSummary, Environment } from "./bindings";
import { environmentMeta, type EnvironmentMeta } from "./environments";

// WHAT:  Which connection a tab belongs to, as something the tab strip can draw:
//        a colour, a name, an environment and where inside the server it points.
// WHY:   Several connections can be open at once and their tabs sit in one strip.
//        A table called `users` on staging and one on production must never be
//        mistaken for each other, so every tab carries its connection's colour and
//        name — the colour the user chose, or a stable one derived from the id.
// HOW:   The colour classes are written out in full (never composed) so Tailwind
//        sees every one; `satisfies` makes a new `ConnectionColor` fail to compile
//        until it has them. Production and staging add their environment wash on
//        top, from the environment registry.
// WHERE: src/features/shell/TabIdentity.tsx, src/features/shell/TabBar.tsx

export interface IdentityClasses {
  /// Solid: the stripe across a tab and the dot in its chip.
  stripe: string;
  /// Faint wash behind an inactive tab / the active one.
  tint: string;
  tintActive: string;
}

export const IDENTITY_CLASSES = {
  blue: { stripe: "bg-conn-blue", tint: "bg-conn-blue/8", tintActive: "bg-conn-blue/16" },
  teal: { stripe: "bg-conn-teal", tint: "bg-conn-teal/8", tintActive: "bg-conn-teal/16" },
  green: { stripe: "bg-conn-green", tint: "bg-conn-green/8", tintActive: "bg-conn-green/16" },
  amber: { stripe: "bg-conn-amber", tint: "bg-conn-amber/8", tintActive: "bg-conn-amber/16" },
  orange: { stripe: "bg-conn-orange", tint: "bg-conn-orange/8", tintActive: "bg-conn-orange/16" },
  red: { stripe: "bg-conn-red", tint: "bg-conn-red/8", tintActive: "bg-conn-red/16" },
  pink: { stripe: "bg-conn-pink", tint: "bg-conn-pink/8", tintActive: "bg-conn-pink/16" },
  violet: { stripe: "bg-conn-violet", tint: "bg-conn-violet/8", tintActive: "bg-conn-violet/16" },
  gray: { stripe: "bg-conn-gray", tint: "bg-conn-gray/8", tintActive: "bg-conn-gray/16" },
} satisfies Record<ConnectionColor, IdentityClasses>;

/// Colours handed to a connection the user never coloured. Red and gray are left
/// out: red already means "production" and gray means "nothing chosen".
const FALLBACK_COLORS: readonly ConnectionColor[] = ["blue", "teal", "green", "amber", "orange", "pink", "violet"];

/// FNV-1a over the id: the same connection always lands on the same colour.
function hash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    h = Math.imul(h ^ text.charCodeAt(i), 0x01000193);
  }
  return h >>> 0;
}

/// The colour a connection's tabs wear: the one picked on the connections page,
/// else one derived from its id so two uncoloured connections still differ.
export function identityColor(connection: Pick<ConnectionSummary, "id" | "color">): ConnectionColor {
  if (connection.color !== null) return connection.color;
  return FALLBACK_COLORS[hash(connection.id) % FALLBACK_COLORS.length] ?? "blue";
}

export interface ConnectionIdentity {
  connectionId: string;
  name: string;
  environment: Environment;
  environmentMeta: EnvironmentMeta;
  readOnly: boolean;
  color: ConnectionColor;
  classes: IdentityClasses;
  /// The database the session is attached to (the file, for SQLite-like engines).
  database: string | null;
  /// The connection record is gone (deleted while its tab stayed open).
  missing: boolean;
}

export function connectionIdentity(connection: ConnectionSummary, database: string | null): ConnectionIdentity {
  const color = identityColor(connection);
  return {
    connectionId: connection.id,
    name: connection.name,
    environment: connection.environment,
    environmentMeta: environmentMeta(connection.environment),
    readOnly: connection.readOnly,
    color,
    classes: IDENTITY_CLASSES[color],
    database: database !== null && database.trim() !== "" ? database : null,
    missing: false,
  };
}

/// What a tab whose connection no longer exists shows instead of a name.
export function missingIdentity(connectionId: string): ConnectionIdentity {
  return {
    connectionId,
    name: "Removed connection",
    environment: "none",
    environmentMeta: environmentMeta("none"),
    readOnly: false,
    color: "gray",
    classes: IDENTITY_CLASSES.gray,
    database: null,
    missing: true,
  };
}

/// "name · database", the chip's text.
export function identityLabel(identity: ConnectionIdentity): string {
  return identity.database === null ? identity.name : `${identity.name} · ${identity.database}`;
}

/// The tooltip's lines: connection (+ environment), database, schema — only those that apply.
export function identityLines(identity: ConnectionIdentity, schema: string | null): string[] {
  const env = identity.environment === "none" ? "" : ` (${identity.environmentMeta.label})`;
  const lines = [`Connection: ${identity.name}${env}${identity.readOnly ? ", read-only" : ""}`];
  if (identity.database !== null) lines.push(`Database: ${identity.database}`);
  if (schema !== null && schema !== "") lines.push(`Schema: ${schema}`);
  return lines;
}
