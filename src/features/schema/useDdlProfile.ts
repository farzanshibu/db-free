// SOT: ddl-profile-cache, use-ddl-profile, schema-form-capabilities
import { useEffect } from "react";
import { create } from "zustand";
import type { DdlProfile } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";

// WHAT:  The schema forms' per-engine catalogue (type picker, what ALTER can do,
//        caveats), fetched once per connection and shared by the sidebar menu and
//        every form.
// WHY:   The sidebar greys out "Duplicate table…" for an engine that cannot, and
//        the forms need the same answer; asking once per session keeps them in step
//        and the menu instant.
// HOW:   `useDdlProfile(connectionId)` returns the cached profile (undefined while
//        it loads or if the call failed, which the caller treats as not known yet: the
//        menu stays enabled and the form reports the engine's own refusal). The
//        profile is static per engine, so it is dropped only when the connection is.
// WHERE: src-tauri/src/integrations/ddl/ (profile), src-tauri/src/commands/schema.rs
interface ProfileStore {
  profiles: Record<string, DdlProfile>;
  /// Connections a load is in flight or done for, so a render loop cannot refetch.
  asked: Record<string, true>;
  ensure: (connectionId: string) => Promise<void>;
}

const useProfiles = create<ProfileStore>()((set, get) => ({
  profiles: {},
  asked: {},
  ensure: async (connectionId) => {
    if (connectionId in get().asked) return;
    set((s) => ({ asked: { ...s.asked, [connectionId]: true } }));
    try {
      const profile = await ipc("ddl_profile", { connectionId });
      set((s) => ({ profiles: { ...s.profiles, [connectionId]: profile } }));
    } catch {
      // Not worth a toast: the menu stays enabled, and the form that needs the
      // profile reports the engine's own refusal. Forget the attempt so a later
      // mount (after the session is back) asks again.
      set((s) => ({ asked: Object.fromEntries(Object.entries(s.asked).filter(([id]) => id !== connectionId)) }));
    }
  },
}));

export function useDdlProfile(connectionId: string | null): DdlProfile | undefined {
  const profile = useProfiles((s) => (connectionId === null ? undefined : s.profiles[connectionId]));
  const ensure = useProfiles((s) => s.ensure);
  useEffect(() => {
    if (connectionId !== null) void ensure(connectionId);
  }, [connectionId, ensure]);
  return profile;
}
