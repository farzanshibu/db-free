// SOT: note-inspector, sticky-note-panel
import type { ConnectionColor, DiagramNote } from "@/lib/bindings";
import { AppSelect, Toggle, type Option } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";
import { ColorSwatches } from "./ColorSwatches";
import { removeNotes, updateNote } from "./designerModel";
import { InspectorPanel, LabeledRow } from "./InspectorParts";
import { DEFAULT_NOTE_FONT_SIZE, NOTE_FONT_SIZES } from "./palette";
import type { Edit } from "./TableInspector";

const SIZE_OPTIONS: readonly Option<string>[] = NOTE_FONT_SIZES.map((s) => ({ value: String(s.value), label: s.label }));

interface NoteInspectorProps {
  note: DiagramNote;
  edit: Edit;
  onClose: () => void;
}

// WHAT:  The side panel for a sticky note: its text, text size, colour, and whether
//        it is left out of exported images.
export function NoteInspector({ note, edit, onClose }: NoteInspectorProps) {
  const patch = (changes: Partial<DiagramNote>) => {
    edit((b) => updateNote(b, note.id, changes));
  };
  return (
    <InspectorPanel
      title="Note"
      onClose={onClose}
      footer={
        <Button
          size="sm"
          variant="danger-soft"
          className="ml-auto"
          onClick={() => {
            edit((b) => removeNotes(b, new Set([note.id])));
            onClose();
          }}
        >
          <Icon name="trash" size={13} />
          Delete
        </Button>
      }
    >
      <LabeledRow label="Text">
        <Textarea
          value={note.text}
          rows={5}
          placeholder="Write a note…"
          aria-label="Note text"
          className="font-sans text-xs"
          onChange={(event) => {
            patch({ text: event.target.value });
          }}
        />
      </LabeledRow>
      <AppSelect
        label="Text size"
        size="sm"
        value={String(note.fontSize ?? DEFAULT_NOTE_FONT_SIZE)}
        options={SIZE_OPTIONS}
        onChange={(size) => {
          patch({ fontSize: Number(size) });
        }}
      />
      <LabeledRow label="Colour">
        <ColorSwatches
          value={note.color}
          resetLabel="Amber"
          onChange={(color: ConnectionColor | null) => {
            patch({ color });
          }}
        />
      </LabeledRow>
      <Toggle
        label="Hide when exporting"
        description="Leaves this note out of PNG and SVG exports."
        checked={note.hideOnExport}
        onChange={(hideOnExport) => {
          patch({ hideOnExport });
        }}
      />
    </InspectorPanel>
  );
}
