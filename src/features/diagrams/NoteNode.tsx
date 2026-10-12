// SOT: note-node, sticky-note, diagram-annotation
import { createContext, useContext, useEffect, useState } from "react";
import { NodeResizer, type Node, type NodeProps } from "@xyflow/react";
import type { DiagramNote } from "@/lib/bindings";
import { cn } from "@/lib/cn";
import { DEFAULT_NOTE_COLOR, DEFAULT_NOTE_FONT_SIZE, colorVar, inkVar } from "./palette";

// eslint-disable-next-line @typescript-eslint/consistent-type-definitions
export type NoteNodeData = {
  note: DiagramNote;
  editable: boolean;
};

// WHAT:  How a note writes a change (its text, or its size and position after a
//        resize) back to the diagram. Through context so each note's data stays
//        plain values.
export interface NoteActions {
  change: (noteId: string, patch: Partial<DiagramNote>) => void;
}

export const NoteActionsContext = createContext<NoteActions>({ change: () => undefined });

export type NoteFlowNode = Node<NoteNodeData, "note">;

/// How long after the last keystroke the text is written back to the diagram.
const TEXT_DEBOUNCE_MS = 500;

// WHAT:  A sticky note on the designer canvas: a coloured card with editable text,
//        resizable from its edges and corners while selected.
// WHY:   Notes explain a diagram to whoever opens it next. They are drawn from the
//        same palette as table headers, so they follow the theme.
// HOW:   The text lives in local state while typing and is committed after a short
//        pause (and on blur), so each keystroke does not rebuild the whole canvas.
//        The grip bar on top is the drag handle; the text area is `nodrag` so
//        selecting text does not move the note.
export function NoteNode({ data, selected }: NodeProps<NoteFlowNode>) {
  const { note, editable } = data;
  const { change } = useContext(NoteActionsContext);
  const onChange = (patch: Partial<DiagramNote>) => {
    change(note.id, patch);
  };
  const color = note.color ?? DEFAULT_NOTE_COLOR;
  // What is being typed, until it is written back; null = show the note's own text
  // (so a restored version replaces the text without any syncing effect).
  const [draft, setDraft] = useState<string | null>(null);

  useEffect(() => {
    if (draft === null) return;
    const timer = window.setTimeout(() => {
      change(note.id, { text: draft });
      setDraft(null);
    }, TEXT_DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [draft, change, note.id]);

  const commit = () => {
    if (draft === null) return;
    onChange({ text: draft });
    setDraft(null);
  };

  return (
    <div
      className={cn("flex size-full flex-col overflow-hidden rounded-md border shadow-md", selected ? "border-accent ring-1 ring-accent" : "border-border")}
      style={{ background: colorVar(color), color: inkVar(color) }}
    >
      <NodeResizer
        isVisible={editable && selected}
        minWidth={120}
        minHeight={60}
        onResizeEnd={(_, params) => {
          onChange({ x: Math.round(params.x), y: Math.round(params.y), width: Math.round(params.width), height: Math.round(params.height) });
        }}
      />
      <div className="flex h-3.5 shrink-0 cursor-grab items-center justify-center bg-foreground/15 active:cursor-grabbing" aria-hidden="true">
        <span className="h-0.5 w-6 rounded-full bg-current opacity-40" />
      </div>
      <textarea
        className="nodrag nowheel size-full flex-1 resize-none bg-transparent p-2 leading-snug outline-none placeholder:text-current placeholder:opacity-50"
        style={{ fontSize: note.fontSize ?? DEFAULT_NOTE_FONT_SIZE }}
        value={draft ?? note.text}
        placeholder="Note…"
        readOnly={!editable}
        aria-label="Note text"
        onChange={(event) => {
          setDraft(event.target.value);
        }}
        onBlur={commit}
      />
    </div>
  );
}
