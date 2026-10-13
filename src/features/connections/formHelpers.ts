// SOT: connection-form-merge, parsed-connection-merge, ssh-config-host-fill, form-issue-lookup
import type { ConnectionField, ConnectionFieldIssue, ConnectionInput, Engine, SshConfigHost, SshTunnel } from "@/lib/bindings";
import { sharesAdapter, type ParsedConnection } from "@/lib/engines";

// WHAT:  Whether a pasted string should leave the form's engine alone.
// WHY:   A `postgres://` string pasted into a CockroachDB or TimescaleDB form is
//        that engine's string too. The two generic engines are the exception:
//        a Postgres form handed a Supabase address becomes the Supabase preset,
//        which is the point of recognising the host.
export function keepsFormEngine(current: Engine, parsed: Engine): boolean {
  if (current === parsed) return true;
  return sharesAdapter(current, parsed) && current !== "postgres" && current !== "mysql";
}

// WHAT:  The form's input after a connection string was applied to it.
// WHY:   Pasting a URL fills the connection fields; it must not wipe what the URL
//        cannot say: the name, environment, read-only lock, SSH tunnel, colour,
//        favourite, and the tags and folders the user already chose.
// HOW:   `force` switches to the string's engine even when the form's engine
//        disagrees (the "Switch to X" button). A password in the string replaces
//        the typed one; without one the typed password stays.
export function mergeParsed(prev: ConnectionInput, parsed: ParsedConnection, force: boolean): ConnectionInput {
  const engine = !force && keepsFormEngine(prev.engine, parsed.engine) ? prev.engine : parsed.engine;
  const base = parsed.input;
  const sameEngine = engine === prev.engine;
  const tlsFromString = base.extra.tls.caPath !== null || base.extra.tls.certPath !== null || base.extra.tls.keyPath !== null;
  return {
    ...base,
    engine,
    name: prev.name.trim().length > 0 ? prev.name : base.name,
    environment: prev.environment,
    readOnly: prev.readOnly,
    password: base.password ?? prev.password,
    ssh: prev.ssh,
    sshSecret: prev.sshSecret,
    folder: null,
    color: prev.color,
    favorite: prev.favorite,
    extra: {
      ...base.extra,
      tls: tlsFromString ? base.extra.tls : prev.extra.tls,
      tagIds: prev.extra.tagIds,
      folderIds: prev.extra.folderIds,
    },
    auth: sameEngine ? prev.auth : base.auth,
    authSecrets: sameEngine ? prev.authSecrets : base.authSecrets,
  };
}

// WHAT:  The SSH tunnel after picking a host from ~/.ssh/config.
// HOW:   The alias is the host to dial unless `HostName` says otherwise. An
//        `IdentityFile` switches to private-key auth with that path; without one
//        the auth method is left as it was. The pinned host key is dropped:
//        it belonged to the previous server.
export function sshFromConfigHost(current: SshTunnel, host: SshConfigHost): SshTunnel {
  return {
    ...current,
    enabled: true,
    host: host.hostName ?? host.alias,
    port: host.port ?? 22,
    user: host.user ?? current.user,
    auth: host.identityFile !== null ? "private_key" : current.auth,
    keyPath: host.identityFile ?? current.keyPath,
    hostKey: null,
  };
}

// WHAT:  The first message for a field, or null.
export function issueFor(issues: readonly ConnectionFieldIssue[], field: ConnectionField): string | null {
  return issues.find((issue) => issue.field === field)?.message ?? null;
}

// WHAT:  A label for the engine a pasted string was recognised as, for the preview line.
export function describeParsed(parsed: ParsedConnection, engineLabel: string): string {
  const host = parsed.input.host ?? parsed.input.filePath ?? "";
  return host.length > 0 ? `${engineLabel} · ${host}` : engineLabel;
}
