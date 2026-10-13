// SOT: credential-banner, credentials-lost-ui, master-key-file-warning
import { useEffect, useState } from "react";
import type { CredentialStatus } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { ConfirmDialog } from "@/components/global/ConfirmDialog";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

// WHAT:  The two standing warnings about the master key that seals saved passwords:
//        "credentials lost" (the OS keychain entry is gone) and "key kept in a file"
//        (no OS keychain is available on this machine).
// WHY:   A missing keychain entry used to be silent: every password then failed to
//        decrypt with an error that blamed the password. Now the page says what
//        happened and offers the only recovery, which is to clear the passwords
//        that can no longer be read and enter them again. The file fallback is
//        weaker than a keychain, so it carries a visible warning for as long as
//        it is in use.
// HOW:   Asks Rust once on mount; `reset_credentials` refuses unless the key is
//        really lost, so the button cannot wipe working credentials.
// WHERE: src-tauri/src/state.rs (credential_status), src-tauri/src/adapters/keyring.rs
export function CredentialBanner() {
  const refreshConnections = useWorkspace((s) => s.refreshConnections);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [status, setStatus] = useState<CredentialStatus | null>(null);
  const [dismissedFileWarning, setDismissedFileWarning] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const next = await ipc("credential_status");
        if (!cancelled) setStatus(next);
      } catch (raw) {
        if (!cancelled) setError(normalizeError(raw).message);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  const reset = async () => {
    setResetting(true);
    try {
      const next = await ipc("reset_credentials");
      setStatus(next);
      setConfirming(false);
      await refreshConnections();
      showInfo("Saved passwords cleared. Enter each password again when you connect.");
    } catch (raw) {
      setError(normalizeError(raw).message);
      setConfirming(false);
    } finally {
      setResetting(false);
    }
  };

  if (status === null && error === null) return null;

  return (
    <div className="mb-3 flex flex-col gap-2">
      {error !== null ? (
        <Alert variant="danger">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle>Could not check the credential store</AlertTitle>
            <AlertDescription className="selectable">{error}</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      {status?.lost === true ? (
        <Alert variant="danger">
          <AlertIndicator />
          <AlertContent className="gap-2">
            <AlertTitle>Credentials lost</AlertTitle>
            <AlertDescription>
              The OS keychain no longer holds the key that seals your saved passwords, so {status.lostSecrets} stored password{status.lostSecrets === 1 ? "" : "s"} can't be decrypted. DB Free did not create a replacement
              key, because that would hide the problem. Clear the unreadable passwords to continue; you will enter each one again.
            </AlertDescription>
            <div>
              <Button variant="danger" size="sm" onClick={() => { setConfirming(true); }}>
                Clear saved passwords…
              </Button>
            </div>
          </AlertContent>
        </Alert>
      ) : null}
      {status !== null && !status.lost && status.storage === "file" && !dismissedFileWarning ? (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent className="gap-2">
            <AlertTitle>No OS keychain available</AlertTitle>
            <AlertDescription>
              The key that seals your saved passwords is kept in a file readable only by you
              {status.keyPath !== null ? <> (<span className="selectable break-all font-mono">{status.keyPath}</span>)</> : null}. Anyone who can read that file can decrypt your saved passwords. Install or unlock a keychain (Secret Service on Linux) to move it there.
            </AlertDescription>
            <div>
              <Button variant="tertiary" size="sm" onClick={() => { setDismissedFileWarning(true); }}>
                Dismiss
              </Button>
            </div>
          </AlertContent>
        </Alert>
      ) : null}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title="Clear saved passwords?"
        description="The passwords (and SSH passwords, and the AI key) that can't be decrypted are removed. Connections stay; you enter the passwords again."
        confirmLabel="Clear passwords"
        pending={resetting}
        onConfirm={reset}
      />
    </div>
  );
}
