// SOT: editor-diagnostics, error-squiggle, error-hover
import { StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, hoverTooltip, type DecorationSet } from "@codemirror/view";

// WHAT:  The red squiggle under the spot where the database said a statement
//        went wrong, with the server's message on hover.
// WHY:   "syntax error at or near FORM" is much easier to act on when FORM is
//        underlined in the text than when the message has to be matched against
//        the query by eye.
// HOW:   One diagnostic at a time (a failed run reports its first failure). It
//        lives in a state field so it follows the text, and any edit clears it:
//        a squiggle on text that has changed is misinformation. The range is
//        the word at the reported offset, or one character when it falls on
//        punctuation.
// WHERE: src/features/editor/SqlEditor.tsx, src/features/editor/QueryPane.tsx

export interface EditorDiagnostic {
  /// Document offset the server pointed at.
  at: number;
  message: string;
  hint: string | null;
  detail: string | null;
}

interface Placed extends EditorDiagnostic {
  from: number;
  to: number;
}

/// `null` clears the diagnostic.
export const setDiagnostic = StateEffect.define<EditorDiagnostic | null>();

const squiggle = Decoration.mark({ class: "cm-diagnostic-error" });

const WORD = /[\p{L}\p{N}_$]/u;

function placed(doc: string, diagnostic: EditorDiagnostic): Placed {
  const length = doc.length;
  const at = Math.min(Math.max(diagnostic.at, 0), length);
  let from = at;
  let to = at;
  while (to < length && WORD.test(doc.charAt(to))) to += 1;
  if (to === at) {
    // On punctuation or whitespace: underline the character there, or the one before at the very end.
    if (at < length) to = at + 1;
    else if (at > 0) from = at - 1;
  } else {
    while (from > 0 && WORD.test(doc.charAt(from - 1))) from -= 1;
  }
  return { ...diagnostic, from, to: Math.max(to, from) };
}

export const diagnosticField = StateField.define<Placed | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setDiagnostic)) return effect.value === null ? null : placed(tr.newDoc.toString(), effect.value);
    }
    return tr.docChanged ? null : value;
  },
  provide: (field) =>
    EditorView.decorations.from(field, (value): DecorationSet =>
      value === null || value.to <= value.from ? Decoration.none : Decoration.set([squiggle.range(value.from, value.to)]),
    ),
});

// WHAT:  The hover card: the message, then the server's hint and detail.
const hover = hoverTooltip((view, pos) => {
  const found = view.state.field(diagnosticField, false);
  if (found === null || found === undefined || pos < found.from || pos > found.to) return null;
  return {
    pos: found.from,
    end: found.to,
    above: true,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-diagnostic-card";
      const lines: [string, string][] = [["message", found.message]];
      if (found.hint !== null) lines.push(["hint", `Hint: ${found.hint}`]);
      if (found.detail !== null) lines.push(["detail", found.detail]);
      for (const [kind, text] of lines) {
        const line = document.createElement("div");
        line.className = `cm-diagnostic-${kind}`;
        line.textContent = text;
        dom.append(line);
      }
      return { dom };
    },
  };
});

export function diagnosticsExtension(): Extension {
  return [diagnosticField, hover];
}
