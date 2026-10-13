// SOT: save-folder-field, save-query-folder-slot, inline-folder-create
import { useEffect } from "react";
import { useLibrary } from "@/stores/library";
import { Label } from "@/components/ui/label";
import { FolderPicker } from "@/features/documents/LibraryDialogs";

// WHAT:  The "Folder" field of the save-query dialog: pick where the new saved query
//        goes, or make a folder right there ("New folder") and file it into that.
// WHY:   Saved queries live in nested folders; choosing the folder is part of naming
//        the query, and a dialog that sent you away to make one first would lose the
//        name you had typed.
// HOW:   Rendered in the dialog's `extra` slot (src/features/editor/SaveQueryDialog.tsx).
//        The choice is held in the library store (`saveFolder`); `workspace.saveQuery`
//        sends it with a new query so the server checks the name against that folder
//        and files the query there in the same write. It is cleared when the field
//        goes away, so a later save never lands in a folder picked earlier.
// WHERE: src/stores/library.ts (saveFolder), src/stores/workspace.ts (saveQuery)
export function SaveFolderField() {
  const folderId = useLibrary((s) => s.saveFolder);
  const setSaveFolder = useLibrary((s) => s.setSaveFolder);
  const loaded = useLibrary((s) => s.loaded);
  const loadLibrary = useLibrary((s) => s.loadLibrary);

  useEffect(() => {
    if (!loaded) void loadLibrary().catch(() => undefined);
  }, [loaded, loadLibrary]);
  useEffect(() => () => setSaveFolder(null), [setSaveFolder]);

  return (
    <div className="flex flex-col gap-1.5">
      <Label>Folder</Label>
      <FolderPicker scope="saved_query" value={folderId} onChange={setSaveFolder} />
    </div>
  );
}
