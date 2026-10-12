// SOT: ai-model-lists-hook, model-choice-storage, model-picker-state
import { useCallback, useEffect, useRef, useState } from "react";
import type { AiModelList, AiProfile, AppSettings, ModelChoice } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { readStored, writeStored } from "@/lib/storage";
import { ipcMessage } from "./ipcMessage";

// WHAT:  The models each configured provider offers, and which one a chat is using.
// WHY:   The picker in the message box lists every provider's models together, so
//        the lists are fetched once per provider (from the server's cache unless
//        a refresh is asked for) and shared by everything that shows them.
// HOW:   A provider is asked only when it can answer: it has a key, or it is a
//        local Ollama. A failure is kept per provider, so one broken endpoint
//        does not hide the others' models.
// WHERE: src/features/chat/ModelPicker.tsx, src-tauri/src/commands/ai.rs (ai_models)

const PROFILE_KEY = "db-free.ai.profile";
const MODEL_KEY = "db-free.ai.model";

/// A provider that can list models right now.
export function canListModels(profile: AiProfile): boolean {
  return profile.hasApiKey || profile.provider === "ollama";
}

export interface AiModelsState {
  lists: Record<string, AiModelList>;
  errors: Record<string, string>;
  loading: ReadonlySet<string>;
  refresh: (profile: AiProfile) => void;
}

export function useAiModels(profiles: readonly AiProfile[]): AiModelsState {
  const [lists, setLists] = useState<Record<string, AiModelList>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState<ReadonlySet<string>>(new Set());
  const asked = useRef(new Set<string>());

  const load = useCallback(async (profile: AiProfile, refresh: boolean) => {
    setLoading((prev) => new Set(prev).add(profile.id));
    try {
      const list = await ipc("ai_models", { profile, refresh });
      setLists((prev) => ({ ...prev, [profile.id]: list }));
      setErrors((prev) => Object.fromEntries(Object.entries(prev).filter(([id]) => id !== profile.id)));
    } catch (raw) {
      setErrors((prev) => ({ ...prev, [profile.id]: ipcMessage(raw) }));
    } finally {
      setLoading((prev) => {
        const next = new Set(prev);
        next.delete(profile.id);
        return next;
      });
    }
  }, []);

  useEffect(() => {
    for (const profile of profiles) {
      if (!canListModels(profile)) continue;
      // A changed address or a newly added key is a new question.
      const signature = `${profile.id}|${profile.provider}|${profile.baseUrl ?? ""}|${profile.hasApiKey ? 1 : 0}`;
      if (asked.current.has(signature)) continue;
      asked.current.add(signature);
      void load(profile, false);
    }
  }, [profiles, load]);

  const refresh = useCallback(
    (profile: AiProfile) => {
      void load(profile, true);
    },
    [load],
  );

  return { lists, errors, loading, refresh };
}

/// The model the settings pick when a chat has not chosen another.
export function defaultChoice(settings: AppSettings | null): ModelChoice | null {
  if (settings === null) return null;
  const profile = settings.ai.defaultProfile;
  return profile === null || settings.ai.model.trim().length === 0 ? null : { profileId: profile, model: settings.ai.model };
}

/// The choice remembered from last time, if its provider still exists.
export function storedChoice(settings: AppSettings | null): ModelChoice | null {
  const profileId = readStored(PROFILE_KEY);
  const model = readStored(MODEL_KEY);
  if (settings === null || profileId === null || model === null || model.length === 0) return null;
  return settings.ai.profiles.some((p) => p.id === profileId) ? { profileId, model } : null;
}

export function rememberChoice(choice: ModelChoice): void {
  writeStored(PROFILE_KEY, choice.profileId);
  writeStored(MODEL_KEY, choice.model);
}
