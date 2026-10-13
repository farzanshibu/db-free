// SOT: organize-connection-dialog, assign-tags-folders-dialog
import { useState } from "react";
import type { ConnectionSummary } from "@/lib/bindings";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { FolderPicker, TagPicker } from "./OrgPicker";
import { useConnectionOrg } from "./useConnectionOrg";

// WHAT:  "Tags and folders…" for one connection: pick any number of each, Save.
// WHY:   Filing a connection should not need the full editor (and must not
//        disturb a live session), so this writes only the two membership lists.
// HOW:   Edits a draft; Save sends both lists in one `set_connection_org` call.
export function OrganizeDialog({ connection, onClose }: { connection: ConnectionSummary; onClose: () => void }) {
  const assign = useConnectionOrg((s) => s.assign);
  const [tagIds, setTagIds] = useState<string[]>(connection.extra.tagIds);
  const [folderIds, setFolderIds] = useState<string[]>(connection.extra.folderIds);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    const saved = await assign(connection.id, { tagIds, folderIds });
    setSaving(false);
    if (saved !== null) onClose();
  };

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>Tags and folders</DialogTitle>
          <DialogDescription>{connection.name}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          <FolderPicker selected={folderIds} onChange={setFolderIds} />
          <TagPicker selected={tagIds} onChange={setTagIds} />
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>
            Cancel
          </Button>
          <Button pending={saving} onClick={() => void save()}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
