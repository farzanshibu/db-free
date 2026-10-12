// SOT: ai-settings-section, ai-provider-profiles-ui, ai-model-fetch-ui, ai-data-access-setting, ai-autonomy-setting
import { useCallback, useEffect, useState, type ReactNode } from "react";
import type {
  AgentAutonomy,
  AgentSkill,
  AiDataAccess,
  AiKeyChange,
  AiModelList,
  AiProfile,
  AiProvider,
  AiSettings,
  AiTestResult,
} from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Field, Segmented, Toggle } from "@/components/global/Field";
import { DATA_ACCESS_OPTIONS } from "@/features/chat/access";
import { relativeTime } from "@/features/chat/chatMessages";
import { ipcMessage } from "@/features/chat/ipcMessage";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";

// WHAT:  Settings → AI: the providers you have configured (each with its own
//        key), which one and which model is the default, what the assistant may
//        see, and how far it may act on its own.
// WHY:   Several providers can be live at once, and the chat picks a model per
//        message from all of them. Keys are the sensitive part: they go to the
//        OS-keychain-sealed store and never come back, so this section handles
//        them apart from the rest of the preferences page.
// HOW:   The section keeps its own draft of the assistant's settings and saves
//        it with its own button (`ai_settings_save`), together with any keys
//        typed. The general Save dock does not carry these (the server keeps the
//        assistant's settings out of that save), so the two can never overwrite
//        each other. Test and Refresh work on the form as typed, before saving.
// WHERE: src/features/settings/SettingsPage.tsx (renders it),
//        src-tauri/src/services/ai_profiles.rs, src-tauri/src/commands/ai.rs

type RealProvider = Exclude<AiProvider, "none">;

const PROVIDERS: readonly { value: RealProvider; label: string; endpoint: string; keyHint: string }[] = [
  { value: "anthropic", label: "Anthropic", endpoint: "https://api.anthropic.com", keyHint: "sk-ant-…" },
  { value: "openai", label: "OpenAI", endpoint: "https://api.openai.com/v1", keyHint: "sk-…" },
  { value: "google", label: "Google Gemini", endpoint: "https://generativelanguage.googleapis.com", keyHint: "AIza…" },
  { value: "openrouter", label: "OpenRouter", endpoint: "https://openrouter.ai/api/v1", keyHint: "sk-or-…" },
  { value: "ollama", label: "Ollama (local)", endpoint: "http://127.0.0.1:11434", keyHint: "" },
];

const AUTONOMY: readonly { value: AgentAutonomy; label: string }[] = [
  { value: "read_only", label: "Read only" },
  { value: "ask_on_write", label: "Ask before writing" },
  { value: "full", label: "Write without asking" },
];

/// The tool-step limit's range; mirrors MIN/MAX/DEFAULT_AGENT_STEPS in src-tauri/src/model/settings.rs.
const STEP_LIMIT = { min: 1, max: 100, fallback: 12 } as const;

function clampSteps(steps: number): number {
  return Number.isFinite(steps) ? Math.min(STEP_LIMIT.max, Math.max(STEP_LIMIT.min, Math.round(steps))) : STEP_LIMIT.fallback;
}

function providerInfo(provider: AiProvider) {
  return PROVIDERS.find((p) => p.value === provider);
}

function SectionRow({ title, body, children }: { title: string; body: ReactNode; children: ReactNode }) {
  return (
    <Card className="rounded-lg glass-card border-border/40 px-3 py-2 shadow-xs">
      <CardContent className="flex w-full flex-row items-center gap-4 p-0">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold tracking-tight text-foreground">{title}</p>
          <p className="mt-0.5 text-xs text-muted">{body}</p>
        </div>
        <div className="shrink-0">{children}</div>
      </CardContent>
    </Card>
  );
}

/// What makes two drafts the same save: everything the server stores, nothing it derives.
function comparable(ai: AiSettings): string {
  return JSON.stringify({
    dataAccess: ai.dataAccess,
    autonomy: ai.autonomy,
    defaultProfile: ai.defaultProfile,
    model: ai.model,
    maxSteps: ai.maxSteps,
    profiles: ai.profiles.map((p) => [p.id, p.provider, p.label, p.baseUrl]),
  });
}

/// Changes whenever the saved settings do, so the body starts over from them.
function savedSignature(ai: AiSettings): string {
  return `${comparable(ai)}|${ai.profiles.map((p) => (p.hasApiKey ? 1 : 0)).join("")}`;
}

export function AiSection() {
  const settings = useWorkspace((s) => s.settings);
  if (settings === null) return null;
  return <AiBody key={savedSignature(settings.ai)} saved={settings.ai} />;
}

interface ModelsState {
  list: AiModelList | null;
  error: string | null;
  loading: boolean;
}

const NO_MODELS: ModelsState = { list: null, error: null, loading: false };

function AiBody({ saved }: { saved: AiSettings }) {
  const loadSettings = useWorkspace((s) => s.loadSettings);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [draft, setDraft] = useState<AiSettings>(saved);
  /// Keys typed since the last save, by profile id.
  const [typedKeys, setTypedKeys] = useState<Record<string, string>>({});
  /// Profiles whose stored key is to be removed on save.
  const [clearedKeys, setClearedKeys] = useState<ReadonlySet<string>>(new Set());
  const [models, setModels] = useState<Record<string, ModelsState>>({});
  const [tests, setTests] = useState<Record<string, AiTestResult | "pending">>({});
  const [adding, setAdding] = useState<RealProvider>("anthropic");
  const [saving, setSaving] = useState(false);
  const [skills, setSkills] = useState<AgentSkill[]>([]);

  // Compiled into the binary, so this is a cheap read and never fails in a way
  // worth surfacing — an empty list just hides the row's detail.
  useEffect(() => {
    void (async () => {
      try {
        setSkills(await ipc("agent_skills"));
      } catch {
        setSkills([]);
      }
    })();
  }, []);

  const dirty = comparable(draft) !== comparable(saved) || Object.values(typedKeys).some((k) => k.trim().length > 0) || clearedKeys.size > 0;
  const defaultProfile = draft.profiles.find((p) => p.id === draft.defaultProfile) ?? null;

  const patchProfile = (id: string, partial: Partial<AiProfile>) => {
    setDraft((d) => ({ ...d, profiles: d.profiles.map((p) => (p.id === id ? { ...p, ...partial } : p)) }));
  };

  const addProfile = () => {
    const info = providerInfo(adding);
    const profile: AiProfile = {
      id: `p-${crypto.randomUUID().slice(0, 8)}`,
      provider: adding,
      label: info?.label ?? adding,
      baseUrl: null,
      hasApiKey: false,
    };
    setDraft((d) => ({
      ...d,
      profiles: [...d.profiles, profile],
      defaultProfile: d.defaultProfile ?? profile.id,
      // Dropping a provider in with no model leaves the default half-set.
      model: d.defaultProfile === null ? "" : d.model,
    }));
  };

  const removeProfile = (id: string) => {
    setDraft((d) => {
      const remaining = d.profiles.filter((p) => p.id !== id);
      const stillDefault = d.defaultProfile === id ? (remaining[0]?.id ?? null) : d.defaultProfile;
      return { ...d, profiles: remaining, defaultProfile: stillDefault, model: stillDefault === d.defaultProfile ? d.model : "" };
    });
    setTypedKeys((k) => Object.fromEntries(Object.entries(k).filter(([key]) => key !== id)));
    setClearedKeys((c) => new Set([...c].filter((key) => key !== id)));
  };

  const loadModels = useCallback(async (profile: AiProfile, typedKey: string, refresh: boolean) => {
    setModels((m) => ({ ...m, [profile.id]: { list: m[profile.id]?.list ?? null, error: null, loading: true } }));
    try {
      const list = await ipc("ai_models", {
        profile,
        refresh,
        ...(typedKey.trim().length > 0 ? { apiKey: typedKey.trim() } : {}),
      });
      setModels((m) => ({ ...m, [profile.id]: { list, error: null, loading: false } }));
    } catch (raw) {
      setModels((m) => ({ ...m, [profile.id]: { list: m[profile.id]?.list ?? null, error: ipcMessage(raw), loading: false } }));
    }
  }, []);

  // Cached lists for the providers that can answer, so the model menu is ready.
  useEffect(() => {
    for (const profile of saved.profiles) {
      if (profile.hasApiKey || profile.provider === "ollama") void loadModels(profile, "", false);
    }
  }, [saved.profiles, loadModels]);

  const test = (profile: AiProfile) => {
    setTests((t) => ({ ...t, [profile.id]: "pending" }));
    void (async () => {
      try {
        const typed = typedKeys[profile.id] ?? "";
        const result = await ipc("ai_test", {
          profile,
          ...(typed.trim().length > 0 ? { apiKey: typed.trim() } : {}),
          ...(draft.defaultProfile === profile.id && draft.model.trim().length > 0 ? { model: draft.model.trim() } : {}),
        });
        setTests((t) => ({ ...t, [profile.id]: result }));
      } catch (raw) {
        setTests((t) => ({ ...t, [profile.id]: { ok: false, message: ipcMessage(raw), modelCount: 0, elapsedMs: 0 } }));
      }
    })();
  };

  const reset = () => {
    setDraft(saved);
    setTypedKeys({});
    setClearedKeys(new Set());
  };

  const save = () => {
    if (draft.defaultProfile !== null && draft.model.trim().length === 0) {
      showError("Pick a default model, or choose no default provider.");
      return;
    }
    const keys: AiKeyChange[] = [
      ...Object.entries(typedKeys)
        .filter(([id, key]) => key.trim().length > 0 && draft.profiles.some((p) => p.id === id))
        .map(([profileId, key]) => ({ profileId, key: key.trim() })),
      ...[...clearedKeys]
        .filter((id) => !(typedKeys[id] ?? "").trim() && draft.profiles.some((p) => p.id === id))
        .map((profileId) => ({ profileId, key: "" })),
    ];
    setSaving(true);
    void (async () => {
      try {
        await ipc("ai_settings_save", { ai: draft, keys });
        await loadSettings();
        showInfo("AI settings saved.");
      } catch (raw) {
        showError(ipcMessage(raw));
      } finally {
        setSaving(false);
      }
    })();
  };

  const defaultModels = defaultProfile === null ? [] : (models[defaultProfile.id]?.list?.models ?? []);
  const accessInfo = DATA_ACCESS_OPTIONS.find((o) => o.value === draft.dataAccess);

  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">AI (bring your own key)</h2>
      <p className="text-xs text-muted">
        A database assistant that reads your schema, writes and runs queries, draws charts and builds dashboards, workflows and diagrams.
        What it may see is your choice below. Keys are encrypted at rest and never shown again.
      </p>

      <SectionRow title="What it may see" body={accessInfo?.description ?? ""}>
        <Segmented<AiDataAccess>
          label="Data access"
          value={draft.dataAccess}
          options={DATA_ACCESS_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
          onChange={(dataAccess) => { setDraft((d) => ({ ...d, dataAccess })); }}
        />
      </SectionRow>

      <div className="flex items-center justify-between pt-1">
        <h3 className="text-xs font-semibold text-foreground">Providers</h3>
        <div className="flex items-center gap-2">
          <AppSelect<RealProvider>
            ariaLabel="Provider to add"
            value={adding}
            options={PROVIDERS.map((p) => ({ value: p.value, label: p.label }))}
            onChange={setAdding}
            className="w-44"
            size="sm"
          />
          <Button size="sm" variant="secondary" onClick={addProfile} disabled={draft.profiles.length >= 12}>
            <Icon name="plus" size={12} />
            Add provider
          </Button>
        </div>
      </div>

      {draft.profiles.length === 0 ? (
        <Card className="rounded-lg glass-card border-dashed border-border/50 px-3 py-3 shadow-xs">
          <CardContent className="flex flex-col items-center gap-1 p-0 text-center">
            <Icon name="brain" size={16} className="text-muted" />
            <p className="text-xs font-semibold text-foreground">No provider yet</p>
            <p className="max-w-sm text-xs text-muted">
              Pick Anthropic, OpenAI, Google Gemini, OpenRouter or a local Ollama above and add your key. You can add several and
              choose between their models in the chat.
            </p>
          </CardContent>
        </Card>
      ) : null}

      {draft.profiles.map((profile) => (
        <ProfileCard
          key={profile.id}
          profile={profile}
          isDefault={draft.defaultProfile === profile.id}
          typedKey={typedKeys[profile.id] ?? ""}
          clearing={clearedKeys.has(profile.id)}
          models={models[profile.id] ?? NO_MODELS}
          test={tests[profile.id]}
          onPatch={(partial) => { patchProfile(profile.id, partial); }}
          onTypeKey={(key) => {
            setTypedKeys((k) => ({ ...k, [profile.id]: key }));
            if (key.trim().length > 0) setClearedKeys((c) => new Set([...c].filter((id) => id !== profile.id)));
          }}
          onClearKey={(clear) => {
            setClearedKeys((c) => {
              const next = new Set(c);
              if (clear) next.add(profile.id);
              else next.delete(profile.id);
              return next;
            });
            if (clear) setTypedKeys((k) => ({ ...k, [profile.id]: "" }));
          }}
          onMakeDefault={() => { setDraft((d) => ({ ...d, defaultProfile: profile.id, model: d.defaultProfile === profile.id ? d.model : "" })); }}
          onRemove={() => { removeProfile(profile.id); }}
          onTest={() => { test(profile); }}
          onRefresh={() => { void loadModels(profile, typedKeys[profile.id] ?? "", true); }}
        />
      ))}

      {defaultProfile !== null ? (
        <SectionRow
          title="Default model"
          body={`Used for new messages unless you pick another in the chat. It runs on ${defaultProfile.label}.`}
        >
          <div className="flex items-center gap-1.5">
            <Field
              label=""
              value={draft.model}
              onChange={(model) => { setDraft((d) => ({ ...d, model })); }}
              placeholder="model id"
              className="w-64 [&_label]:hidden"
              mono
            />
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="secondary" disabled={defaultModels.length === 0} aria-label="Pick from the provider's models">
                  <Icon name="list" size={13} />
                  Models
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="max-h-72 w-64">
                {defaultModels.map((model) => (
                  <DropdownMenuItem key={model.id} onSelect={() => { setDraft((d) => ({ ...d, model: model.id })); }}>
                    <span className="min-w-0 flex-1 truncate">{model.label}</span>
                    {model.id === draft.model ? <Icon name="check" size={12} /> : null}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </SectionRow>
      ) : null}

      <SectionRow
        title="What it may do on its own"
        body="Reads always run unattended. A statement that writes or deletes is shown to you for approval first — unless you turn that off here. A read-only connection refuses writes whatever this says. Saving a dashboard, workflow or diagram always asks."
      >
        <AppSelect<AgentAutonomy>
          ariaLabel="Autonomy"
          value={draft.autonomy}
          options={AUTONOMY}
          onChange={(autonomy) => { setDraft((d) => ({ ...d, autonomy })); }}
          className="w-52"
        />
      </SectionRow>

      <SectionRow
        title="Tool-step limit"
        body={`How many model round trips (each can call tools) one message may take before the assistant stops and offers Continue. ${STEP_LIMIT.min}–${STEP_LIMIT.max}, default ${STEP_LIMIT.fallback}. Raise it for long jobs such as auditing a whole schema; lower it to keep a wandering run cheap.`}
      >
        <StepLimitField value={draft.maxSteps} onChange={(maxSteps) => { setDraft((d) => ({ ...d, maxSteps })); }} />
      </SectionRow>

      <SectionRow
        title="Task guides"
        body={
          skills.length > 0
            ? `Loaded on demand rather than sent with every message: ${skills.map((skill) => skill.name).join(", ")}.`
            : "Short playbooks the assistant loads when a task calls for one."
        }
      >
        <span className="text-xs text-muted tabular-nums">{skills.length}</span>
      </SectionRow>

      {dirty ? (
        <div className="pointer-events-none sticky bottom-3 z-10 flex justify-center">
          <div role="status" className="pointer-events-auto flex items-center gap-2 rounded-lg glass-modal py-1 pr-1.5 pl-2.5 shadow-xl">
            <Icon name="info" size={14} className="text-accent" />
            <span className="mr-3 text-xs font-medium text-foreground">Unsaved AI settings</span>
            <Button size="sm" variant="danger-soft" onClick={reset} disabled={saving} className="rounded-lg liquid-hover">
              Discard
            </Button>
            <Button size="sm" pending={saving} onClick={save} className="font-semibold liquid-hover">
              Save AI settings
            </Button>
          </div>
        </div>
      ) : null}
    </>
  );
}

// WHAT:  The step limit as typed text that commits only valid numbers.
// WHY:   Clamping on every keystroke makes "30" unreachable (the "3" is fine, but
//        a cleared field or a "0" on the way to "10" snaps to the minimum under the
//        caret). The draft takes each valid value as it is typed; the text itself
//        is tidied when the field loses focus.
function StepLimitField({ value, onChange }: { value: number; onChange: (steps: number) => void }) {
  const [typing, setTyping] = useState<string | null>(null);
  return (
    <Input
      type="number"
      inputMode="numeric"
      min={STEP_LIMIT.min}
      max={STEP_LIMIT.max}
      step={1}
      aria-label="Tool-step limit"
      value={typing ?? String(value)}
      onChange={(event) => {
        const text = event.target.value;
        setTyping(text);
        if (text.trim() !== "" && Number.isFinite(Number(text))) onChange(clampSteps(Number(text)));
      }}
      onBlur={() => { setTyping(null); }}
      className="w-20 text-right font-mono tabular-nums"
    />
  );
}

interface ProfileCardProps {
  profile: AiProfile;
  isDefault: boolean;
  typedKey: string;
  clearing: boolean;
  models: ModelsState;
  test: AiTestResult | "pending" | undefined;
  onPatch: (partial: Partial<AiProfile>) => void;
  onTypeKey: (key: string) => void;
  onClearKey: (clear: boolean) => void;
  onMakeDefault: () => void;
  onRemove: () => void;
  onTest: () => void;
  onRefresh: () => void;
}

// WHAT:  One provider: its name, address, key, and a way to check it works.
function ProfileCard({
  profile,
  isDefault,
  typedKey,
  clearing,
  models,
  test,
  onPatch,
  onTypeKey,
  onClearKey,
  onMakeDefault,
  onRemove,
  onTest,
  onRefresh,
}: ProfileCardProps) {
  const info = providerInfo(profile.provider);
  const needsKey = profile.provider !== "ollama";
  const hasKey = profile.hasApiKey && !clearing;
  const typed = typedKey.trim().length > 0;
  const keyStatus = typed
    ? "The new key is saved when you save these settings."
    : clearing
      ? "The stored key is removed when you save."
      : hasKey
        ? "A key is stored. Type a new one to replace it."
        : needsKey
          ? "No key yet."
          : "Ollama runs on your machine and needs no key.";

  return (
    <Card className={cn("rounded-lg glass-card px-3 py-2 shadow-xs", isDefault ? "border-accent/40" : "border-border/40")}>
      <CardContent className="flex flex-col gap-2 p-0">
        <div className="flex items-center gap-2">
          <div className="flex size-6 shrink-0 items-center justify-center rounded-md bg-accent/15 text-accent">
            <Icon name="brain" size={14} />
          </div>
          <Field
            label=""
            value={profile.label}
            onChange={(label) => { onPatch({ label }); }}
            placeholder={info?.label ?? "Name"}
            className="max-w-64 flex-1 [&_label]:hidden"
            compact
          />
          <Badge size="sm" variant="secondary" className="h-5 px-2 text-[10px] glass-pill text-muted">
            {info?.label ?? profile.provider}
          </Badge>
          <div className="ml-auto flex items-center gap-1.5">
            {isDefault ? (
              <Badge size="sm" variant="secondary" className="h-5 px-2 text-[10px] text-accent glass-pill">
                <Icon name="star" size={9} />
                Default
              </Badge>
            ) : (
              <Button size="sm" variant="ghost" onClick={onMakeDefault}>
                Make default
              </Button>
            )}
            <Button size="sm" variant="danger-soft" aria-label={`Remove ${profile.label}`} onClick={onRemove}>
              <Icon name="trash" size={12} />
              Remove
            </Button>
          </div>
        </div>

        <div className="grid gap-2 sm:grid-cols-2">
          <Field
            label="Base URL"
            optional
            value={profile.baseUrl ?? ""}
            onChange={(url) => { onPatch({ baseUrl: url.trim().length > 0 ? url : null }); }}
            placeholder={info?.endpoint ?? ""}
            description="Leave empty for the provider's own endpoint. Proxies and remote Ollama go here."
            mono
          />
          {needsKey ? (
            <div className="flex flex-col gap-1.5">
              <Field
                label="API key"
                type="password"
                value={typedKey}
                onChange={onTypeKey}
                placeholder={hasKey ? "••••••••  (stored)" : (info?.keyHint ?? "")}
                description={keyStatus}
                mono
              />
              {profile.hasApiKey ? <Toggle checked={clearing} onChange={onClearKey} label="Remove the stored key on save" /> : null}
            </div>
          ) : (
            <p className="self-end pb-1 text-xs text-muted">{keyStatus}</p>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2 border-t border-border/30 pt-1.5">
          <Button size="sm" variant="secondary" onClick={onTest} disabled={test === "pending"}>
            {test === "pending" ? <Spinner size="sm" /> : <Icon name="plug" size={12} />}
            Test
          </Button>
          <Button size="sm" variant="ghost" onClick={onRefresh} disabled={models.loading}>
            {models.loading ? <Spinner size="sm" /> : <Icon name="refresh" size={12} />}
            Refresh models
          </Button>
          <span className="text-[11px] text-muted">
            {models.list !== null
              ? `${models.list.models.length} model${models.list.models.length === 1 ? "" : "s"}${
                  models.list.fetchedAt !== null ? ` · ${models.list.cached ? "cached" : "fetched"} ${relativeTime(models.list.fetchedAt)}` : ""
                }`
              : "Models are listed once a key is stored."}
          </span>
        </div>
        {test !== undefined && test !== "pending" ? (
          <p className={cn("text-[11px]", test.ok ? "text-success" : "text-danger")}>{test.message}</p>
        ) : null}
        {models.error !== null ? <p className="text-[11px] text-danger">{models.error}</p> : null}
      </CardContent>
    </Card>
  );
}
