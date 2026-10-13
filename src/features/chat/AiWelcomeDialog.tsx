// SOT: ai-welcome-dialog, ai-first-run, ai-data-access-choice
import { useState } from "react";
import type { AiDataAccess, AppSettings } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { readStored, writeStored } from "@/lib/storage";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DATA_ACCESS_OPTIONS } from "./access";
import { ipcMessage } from "./ipcMessage";

// WHAT:  The first-run dialog: what the assistant is, what it can see, and a way
//        to set it up.
// WHY:   The assistant sends your prompt, the schema it looks up and, if you
//        allow it, the rows its queries return to a third party. That deserves a
//        decision made once, up front, in plain words, not a setting found later.
// HOW:   Shown the first time AI is reached without a provider configured, until
//        it is answered. "Set up a provider" records the data-access choice and
//        opens Settings → AI; "Not now" only closes it. Either way it is not
//        shown again (the choice lives in Settings → AI from then on).
// WHERE: src/features/chat/ChatTab.tsx (mounts it), src/features/settings/AiSection.tsx

const SEEN_KEY = "db-free.ai.welcome-seen";

/// Whether the dialog should appear: no provider yet, and not answered before.
export function shouldWelcome(settings: AppSettings | null): boolean {
  return settings !== null && settings.ai.profiles.length === 0 && readStored(SEEN_KEY) === null;
}

export function AiWelcomeDialog({ settings, onClose }: { settings: AppSettings; onClose: () => void }) {
  const goSettingsSection = useWorkspace((s) => s.goSettingsSection);
  const loadSettings = useWorkspace((s) => s.loadSettings);
  const showError = useWorkspace((s) => s.showError);
  const [choice, setChoice] = useState<AiDataAccess>(settings.ai.dataAccess === "off" ? "schema" : settings.ai.dataAccess);
  const [saving, setSaving] = useState(false);

  const finish = () => {
    writeStored(SEEN_KEY, "1");
    onClose();
  };

  const setUp = () => {
    setSaving(true);
    void (async () => {
      try {
        if (choice !== settings.ai.dataAccess) {
          await ipc("ai_settings_save", { ai: { ...settings.ai, dataAccess: choice }, keys: [] });
          await loadSettings();
        }
        finish();
        goSettingsSection("ai");
      } catch (raw) {
        showError(ipcMessage(raw));
      } finally {
        setSaving(false);
      }
    })();
  };

  const keepOff = () => {
    setSaving(true);
    void (async () => {
      try {
        await ipc("ai_settings_save", { ai: { ...settings.ai, dataAccess: "off" }, keys: [] });
        await loadSettings();
        finish();
      } catch (raw) {
        showError(ipcMessage(raw));
      } finally {
        setSaving(false);
      }
    })();
  };

  return (
    <Dialog open onOpenChange={(open) => { if (!open) finish(); }}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <div className="mb-1 flex size-8 items-center justify-center rounded-lg bg-accent/15 text-accent">
            <Icon name="sparkles" size={16} />
          </div>
          <DialogTitle>Meet your database assistant</DialogTitle>
          <DialogDescription>
            Ask questions in plain language. It reads your schema, writes and runs queries, draws charts, and builds
            dashboards, workflows and diagrams you can keep. It asks before it changes anything.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-2">
          <p className="text-xs font-medium text-foreground">What may it see?</p>
          <div className="space-y-2" role="radiogroup" aria-label="Data access">
            {DATA_ACCESS_OPTIONS.filter((option) => option.value !== "off").map((option) => (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={choice === option.value}
                onClick={() => { setChoice(option.value); }}
                className={cn(
                  "flex w-full items-start gap-2 rounded-lg border p-2 text-left liquid-hover",
                  choice === option.value ? "border-accent/50 bg-accent/10" : "border-border/50 hover:bg-surface-secondary/60",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-full border",
                    choice === option.value ? "border-accent" : "border-border",
                  )}
                >
                  {choice === option.value ? <span className="size-1.5 rounded-full bg-accent" /> : null}
                </span>
                <span>
                  <span className="block text-xs font-semibold text-foreground">{option.label}</span>
                  <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">{option.description}</span>
                </span>
              </button>
            ))}
          </div>
          <p className="text-[11px] text-muted">
            You bring your own provider and key; nothing goes through DB Free. You can change this any time in Settings → AI.
          </p>
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" disabled={saving} onClick={keepOff}>
            Keep AI off
          </Button>
          <Button variant="tertiary" disabled={saving} onClick={finish}>
            Not now
          </Button>
          <Button pending={saving} onClick={setUp}>
            Set up a provider
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
