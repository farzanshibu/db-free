// SOT: profile-gate, schema-form-unsupported-notice
import type { ReactNode } from "react";
import type { DdlProfile } from "@/lib/bindings";
import { StatusDialog } from "./StatusDialog";
import { useFormProfile } from "./useFormProfile";

interface ProfileGateProps {
  connectionId: string;
  /// Dialog title while loading, and when the engine has no forms.
  title: string;
  onClose: () => void;
  children: (profile: DdlProfile) => ReactNode;
}

// WHAT:  Holds a schema form back until the engine's catalogue is known, and says
//        so plainly when the engine has no forms at all.
// WHY:   Every form's first render needs the type list and the support flags. An
//        engine the builders do not speak to must get an explanation, not a form
//        that fails on Run.
// HOW:   Render-prop: the form body is only mounted with a loaded, supported profile.
// WHERE: src-tauri/src/integrations/ddl/ (the profile)
export function ProfileGate({ connectionId, title, onClose, children }: ProfileGateProps) {
  const { profile, error, retry } = useFormProfile(connectionId);
  if (profile?.supported === true) return <>{children(profile)}</>;
  if (profile !== null) {
    return <StatusDialog title={title} description={`${profile.dialect} is not supported by the schema forms.`} onClose={onClose} notes={profile.notes} />;
  }
  return <StatusDialog title={title} description="Reading what this engine supports…" onClose={onClose} error={error} onRetry={retry} />;
}
