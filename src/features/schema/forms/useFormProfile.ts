// SOT: form-profile-loader, schema-form-profile-gate
import { useEffect, useState } from "react";
import type { DdlProfile } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";

interface Loaded {
  attempt: number;
  profile: DdlProfile | null;
  error: string | null;
}

export interface FormProfile {
  profile: DdlProfile | null;
  error: string | null;
  retry: () => void;
}

// WHAT:  Loads the engine's schema-form catalogue for the dialog that is open,
//        reporting a failure instead of swallowing it.
// WHY:   `useDdlProfile` (the sidebar's cache) treats a failed load as "not known yet"
//        so a menu stays usable; a form cannot be built from nothing, so it
//        asks itself and can show the reason with a Retry.
// HOW:   Results are tagged with the attempt that produced them, so a late answer
//        to an earlier attempt can never overwrite a newer one.
// WHERE: src/features/schema/forms/ProfileGate.tsx
export function useFormProfile(connectionId: string): FormProfile {
  const [attempt, setAttempt] = useState(0);
  const [loaded, setLoaded] = useState<Loaded | null>(null);

  useEffect(() => {
    const guard = { stale: false };
    void (async () => {
      try {
        const profile = await ipc("ddl_profile", { connectionId });
        if (!guard.stale) setLoaded({ attempt, profile, error: null });
      } catch (raw) {
        if (!guard.stale) setLoaded({ attempt, profile: null, error: errorMessage(normalizeError(raw)) });
      }
    })();
    return () => {
      guard.stale = true;
    };
  }, [connectionId, attempt]);

  const current = loaded !== null && loaded.attempt === attempt ? loaded : null;
  return {
    profile: current?.profile ?? null,
    error: current?.error ?? null,
    retry: () => {
      setAttempt((a) => a + 1);
    },
  };
}
