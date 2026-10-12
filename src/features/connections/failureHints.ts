// SOT: connection-failure-hints, connect-checklist
import type { ConnectionSummary } from "@/lib/bindings";

// WHAT:  The things worth checking after a failed connect, most specific first.
// WHY:   "Connection failed" with the driver's raw text sends people searching.
//        The three base checks always apply; the rest are added when the error
//        text points at them (a refused socket, a TLS handshake, the SSH hop, a
//        rejected password), so the list reads as advice for this failure.
// HOW:   Pure: matches the message against patterns and the connection's
//        settings (is a tunnel on, is it a cloud host), nothing else.
// WHERE: src/features/connections/ConnectionFailedDialog.tsx
export function connectionFailureHints(message: string, connection: ConnectionSummary | null): string[] {
  const text = message.toLowerCase();
  const hints: string[] = [];
  const tunnelled = connection?.ssh.enabled === true;

  if (text.includes("credentials lost")) {
    hints.push("The saved passwords can no longer be decrypted. Clear them from the Connections page and enter each password again.");
  }
  if (tunnelled && text.includes("ssh")) {
    hints.push("The SSH tunnel: check the bastion host, port and user, and that the key or password is right.");
    if (text.includes("host key")) hints.push("The SSH server presented a different host key than the one pinned. If it was re-keyed on purpose, clear the fingerprint on the SSH tab.");
  }
  if (/password|authentication|auth failed|access denied|denied for user|login failed|sasl|permission denied/.test(text)) {
    hints.push("The username and password (or token) are correct for this server, and the user may connect from this machine.");
  }
  if (/ssl|tls|certificate|handshake|verify/.test(text)) {
    hints.push("TLS: try the SSL mode the server expects (Require for most hosted databases), or add its CA file under SSL / TLS.");
  }
  if (/refused|unreachable|timed out|timeout|no route|resolve|dns|lookup|name or service|network/.test(text)) {
    hints.push("The host name and port are right, and a VPN, firewall or IP allow-list is not blocking this machine.");
  }
  if (/ipv6|network is unreachable/.test(text) || connection?.host?.endsWith(".supabase.co") === true) {
    hints.push("Direct Supabase hosts are IPv6-only: on an IPv4 network use the pooler host from the Connect panel.");
  }

  // The base checklist, always last: it is what applies when nothing above matched.
  hints.push("The database server is running and accepts connections.");
  hints.push("The credentials are correct.");
  hints.push("Network and firewall settings allow the connection.");
  return hints;
}
