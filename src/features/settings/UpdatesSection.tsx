// SOT: updates-settings, update-check-ui, update-install-ui
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Toggle } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { runShellAction } from "@/features/shell/shellActions";
import { useUpdates } from "@/features/shell/useUpdates";
import { SettingsRow } from "./SettingsRow";

// WHAT:  Settings → Updates: the running version, whether a newer one has been
//        found (and downloaded), a button to look now, and the launch-check switch.
// WHY:   The check at launch is quiet by design; this is where a person asks
//        explicitly, turns the automatic check off, and finds the release notes
//        and the About dialog.
// HOW:   Shares the update state with the update bar (useUpdates), so a check
//        started here shows in the bar too, and the "Install and restart" button
//        is the same action. The launch-check switch edits the Settings draft.
// WHERE: src/features/shell/useUpdates.ts, src-tauri/src/commands/updates.rs
export function UpdatesSection({ checkOnLaunch, onCheckOnLaunch }: { checkOnLaunch: boolean; onCheckOnLaunch: (on: boolean) => void }) {
  const setAboutOpen = useWorkspace((s) => s.setAboutOpen);
  const phase = useUpdates((s) => s.phase);
  const available = useUpdates((s) => s.available);
  const progress = useUpdates((s) => s.progress);
  const error = useUpdates((s) => s.error);
  const upToDate = useUpdates((s) => s.upToDate);
  const check = useUpdates((s) => s.check);
  const install = useUpdates((s) => s.install);
  const busy = phase === "checking" || phase === "downloading";
  const percent = progress?.total ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null;

  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">Updates</h2>
      <p className="text-xs text-muted">Signed builds are published with each release; the app verifies the signature before installing.</p>
      <SettingsRow title="Check for updates" body="Looks for a newer release and downloads it, so installing is just a restart.">
        <Button size="sm" variant="secondary" pending={busy} onClick={() => void check(false)}>
          <Icon name="refresh" size={13} />
          Check for updates
        </Button>
      </SettingsRow>
      <SettingsRow title="Check at launch" body="Look for a new release when the app starts (at most every few hours).">
        <Toggle checked={checkOnLaunch} onChange={onCheckOnLaunch} label="" />
      </SettingsRow>
      {phase === "downloading" ? (
        <SettingsRow title="Downloading the update" body={percent !== null ? `${String(percent)}% done.` : "Starting the download…"}>
          <Icon name="download" size={14} className="text-accent" />
        </SettingsRow>
      ) : null}
      {phase === "ready" && available !== null ? (
        <SettingsRow title={`Version ${available} is ready`} body="Downloaded and verified. Installing restarts the app.">
          <Button size="sm" onClick={() => void install()}>
            <Icon name="download" size={13} />
            Install and restart
          </Button>
        </SettingsRow>
      ) : null}
      {phase === "idle" && upToDate ? (
        <SettingsRow title="Up to date" body="No release newer than this build has been found.">
          <Icon name="check" size={14} className="text-success" />
        </SettingsRow>
      ) : null}
      {phase === "error" && error !== null ? (
        <SettingsRow title="The last check failed" body={error}>
          <Icon name="alert" size={14} className="text-warning" />
        </SettingsRow>
      ) : null}
      {progress && busy ? (
        <div className="h-1 w-full overflow-hidden rounded-full bg-surface-secondary">
          <div className="h-full rounded-full bg-accent transition-[width] duration-200" style={{ width: `${String(percent ?? 100)}%` }} />
        </div>
      ) : null}
      <SettingsRow title="What's new" body="The changelog, and the notes of a release that is waiting.">
        <Button size="sm" variant="secondary" onClick={() => runShellAction("whats-new")}>
          What&apos;s new
        </Button>
      </SettingsRow>
      <SettingsRow title="About DB Free" body="Version, platform, and where this install keeps its data and logs.">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            setAboutOpen(true);
          }}
        >
          About
        </Button>
      </SettingsRow>
    </>
  );
}
