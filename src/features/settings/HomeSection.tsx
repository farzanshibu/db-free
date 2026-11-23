// SOT: home-settings, show-pinned-setting, show-recents-setting
import { Toggle } from "@/components/global/Field";
import { SettingsRow as Row } from "./SettingsRow";

interface HomeSectionProps {
  showPinned: boolean;
  showRecents: boolean;
  onChange: (partial: { showPinned?: boolean; showRecents?: boolean }) => void;
}

// WHAT:  Settings → Home: whether the Pinned board and the Recents show above the
//        connection list.
// WHY:   Both are conveniences; someone who wants a plain list of connections can
//        switch them off without losing what is pinned or recorded.
// HOW:   Edits the draft's `showPinned` / `showRecents`; Save on the settings dock persists them.
// WHERE: src/features/home/HomeBoard.tsx, AppSettings.showPinned / showRecents
export function HomeSection({ showPinned, showRecents, onChange }: HomeSectionProps) {
  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">Home</h2>
      <Row title="Show pinned on Home" body="The board of pinned connections, tables, saved queries, dashboards, workflows and diagrams.">
        <Toggle checked={showPinned} onChange={(value) => onChange({ showPinned: value })} label="" />
      </Row>
      <Row title="Show recents on Home" body="What you opened lately, as shortcuts. Clear removes them; turning this off only hides them.">
        <Toggle checked={showRecents} onChange={(value) => onChange({ showRecents: value })} label="" />
      </Row>
    </>
  );
}
