// SOT: value-code-editor, redis-string-editor, json-highlighting, codemirror-value-editor
import { useEffect, useRef } from "react";
import { Compartment, EditorState, RangeSetBuilder, type Extension } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, drawSelection, highlightActiveLine, keymap, lineNumbers, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { useResolvedTheme, type ResolvedTheme } from "@/stores/useTheme";

// WHAT:  A CodeMirror text area for one value: a Redis string or a JSON document.
// WHY:   Values are free-form and often large; a plain textarea has no line
//        numbers, no undo history worth the name and no hint where a JSON
//        document went wrong. This is the SQL editor's engine with the SQL
//        removed.
// HOW:   Controlled by `value` (an outside change replaces the document; typing
//        calls `onChange`). `language="json"` (switchable at any time) colours strings, numbers, booleans
//        and keys with a view plugin that scans only what is on screen, so a
//        multi-megabyte document stays responsive.
// WHERE: src/features/keys/StringEditor.tsx, src/features/keys/JsonEditor.tsx
export type ValueLanguage = "json" | "text";

interface ValueCodeEditorProps {
  value: string;
  onChange: (value: string) => void;
  language: ValueLanguage;
  readOnly?: boolean;
  ariaLabel: string;
  /// Called for Mod-s inside the editor (save).
  onSave?: (() => void) | undefined;
}

const THEME_SPEC: Parameters<typeof EditorView.theme>[0] = {
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)", height: "100%", fontSize: "var(--editor-font-size, 13px)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55", overflow: "auto" },
  ".cm-content": { caretColor: "var(--color-accent)", padding: "6px 0" },
  ".cm-line": { padding: "0 10px" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--color-muted)", border: "none", paddingRight: "4px" },
  ".cm-activeLine": { backgroundColor: "var(--color-surface-hover)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--foreground)" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--color-accent)", borderLeftWidth: "2px" },
  ".cm-selectionLayer .cm-selectionBackground": { backgroundColor: "var(--color-selection)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, ::selection": { backgroundColor: "var(--color-selection)" },
  ".cm-json-key": { color: "var(--color-syntax-type)" },
  ".cm-json-string": { color: "var(--color-syntax-string)" },
  ".cm-json-number": { color: "var(--color-syntax-number)" },
  ".cm-json-literal": { color: "var(--color-syntax-keyword)", fontWeight: "600" },
};

const DARK_THEME = EditorView.theme(THEME_SPEC, { dark: true });
const LIGHT_THEME = EditorView.theme(THEME_SPEC, { dark: false });

function editorTheme(theme: ResolvedTheme): Extension {
  return theme === "light" ? LIGHT_THEME : DARK_THEME;
}

const KEY = Decoration.mark({ class: "cm-json-key" });
const STRING = Decoration.mark({ class: "cm-json-string" });
const NUMBER = Decoration.mark({ class: "cm-json-number" });
const LITERAL = Decoration.mark({ class: "cm-json-literal" });

/// A string (and, if a colon follows, it is a key), a literal, or a number.
const JSON_TOKEN = /("(?:\\.|[^"\\\n])*")(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;

function jsonDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    const text = view.state.doc.sliceString(from, to);
    for (const match of text.matchAll(JSON_TOKEN)) {
      const start = from + match.index;
      const quoted = match[1];
      if (quoted !== undefined) {
        builder.add(start, start + quoted.length, match[2] === undefined ? STRING : KEY);
      } else {
        builder.add(start, start + match[0].length, /^[-\d]/.test(match[0]) ? NUMBER : LITERAL);
      }
    }
  }
  return builder.finish();
}

const jsonHighlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = jsonDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) this.decorations = jsonDecorations(update.view);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

function editable(readOnly: boolean): Extension {
  return [EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)];
}

export function ValueCodeEditor({ value, onChange, language, readOnly = false, ariaLabel, onSave }: ValueCodeEditorProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const editableCompartment = useRef(new Compartment());
  const languageCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);
  const appTheme = useResolvedTheme();
  const themeRef = useRef(appTheme);
  const initial = useRef({ value, language, readOnly, ariaLabel });

  useEffect(() => {
    onChangeRef.current = onChange;
    onSaveRef.current = onSave;
  });

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const start = initial.current;
    const view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: start.value,
        extensions: [
          lineNumbers(),
          highlightActiveLine(),
          drawSelection(),
          history(),
          EditorView.lineWrapping,
          EditorView.contentAttributes.of({ "aria-label": start.ariaLabel, spellcheck: "false" }),
          keymap.of([
            {
              key: "Mod-s",
              run: () => {
                onSaveRef.current?.();
                return onSaveRef.current !== undefined;
              },
            },
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
          languageCompartment.current.of(start.language === "json" ? jsonHighlight : []),
          editableCompartment.current.of(editable(start.readOnly)),
          themeCompartment.current.of(editorTheme(themeRef.current)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChangeRef.current(update.state.doc.toString());
          }),
        ],
      }),
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } });
  }, [value]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: editableCompartment.current.reconfigure(editable(readOnly)) });
  }, [readOnly]);

  useEffect(() => {
    viewRef.current?.dispatch({ effects: languageCompartment.current.reconfigure(language === "json" ? jsonHighlight : []) });
  }, [language]);

  useEffect(() => {
    themeRef.current = appTheme;
    viewRef.current?.dispatch({ effects: themeCompartment.current.reconfigure(editorTheme(appTheme)) });
  }, [appTheme]);

  return <div ref={hostRef} className="h-full min-h-0 w-full overflow-hidden" />;
}
