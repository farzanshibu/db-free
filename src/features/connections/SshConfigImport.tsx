// SOT: ssh-config-import, openssh-host-picker
import { useState } from "react";
import type { SshConfigHost } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Spinner } from "@/components/ui/spinner";

// WHAT:  "Import from ~/.ssh/config": pick a host and the tunnel form fills itself
//        (host, port, user, key file).
// WHY:   The bastion is usually already described in the user's OpenSSH config;
//        retyping it is where typos in the key path come from.
// HOW:   The list is read from Rust when the popover opens (a missing config is
//        an empty list, not an error). Wildcard entries are not hosts and never
//        appear. Hosts with an IdentityFile switch the tunnel to key auth.
// WHERE: src-tauri/src/services/connection_files.rs (parse_ssh_config)
export function SshConfigImport({ onPick }: { onPick: (host: SshConfigHost) => void }) {
  const [open, setOpen] = useState(false);
  const [hosts, setHosts] = useState<SshConfigHost[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setHosts(null);
    setError(null);
    try {
      setHosts(await ipc("list_ssh_config_hosts"));
    } catch (raw) {
      setError(normalizeError(raw).message);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) void load();
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm" className="w-fit rounded-md">
          <Icon name="download" size={13} />
          Import from ~/.ssh/config
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-1.5" align="start">
        {error !== null ? <p className="px-2 py-2 text-xs text-danger">{error}</p> : null}
        {error === null && hosts === null ? (
          <div className="flex items-center gap-2 px-2 py-2 text-xs text-muted">
            <Spinner size="sm" /> Reading ~/.ssh/config…
          </div>
        ) : null}
        {hosts !== null && hosts.length === 0 ? <p className="px-2 py-2 text-xs text-muted">No hosts found in ~/.ssh/config. Wildcard entries (Host *) are not listed.</p> : null}
        <ul className="flex max-h-64 flex-col overflow-y-auto" aria-label="SSH hosts">
          {(hosts ?? []).map((host) => (
            <li key={host.alias}>
              <Button
                variant="ghost"
                onClick={() => {
                  onPick(host);
                  setOpen(false);
                }}
                className="h-auto w-full flex-col items-start gap-0.5 rounded-md px-2 py-1 text-left"
              >
                <span className="text-xs font-medium text-foreground">{host.alias}</span>
                <span className="w-full truncate font-mono text-[11px] text-muted">
                  {host.user !== null ? `${host.user}@` : ""}
                  {host.hostName ?? host.alias}
                  {host.port !== null ? `:${host.port}` : ""}
                  {host.identityFile !== null ? ` · ${host.identityFile}` : ""}
                </span>
              </Button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}
