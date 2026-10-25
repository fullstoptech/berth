// window.berth.editor: CodeMirror 6 hosted in the webview (PLAN 7.5, ADR 0004).
// Rust owns the text; this file holds a copy while the editor is open. Colours
// come from the design tokens, so the light and dark themes follow tokens.css.

import "./editor.css";
import { yaml } from "@codemirror/lang-yaml";
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { lintGutter, setDiagnostics, type Diagnostic } from "@codemirror/lint";
import { Annotation, Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { basicSetup } from "codemirror";
import { emit, isDark, onThemeChange, sealApi, whenElement } from "./common";

type ThemeName = "light" | "dark";
type Lang = "yaml" | "dockerfile" | "text";

interface CreateOptions {
  lang: Lang;
  value: string;
  readOnly: boolean;
  theme: ThemeName;
}

interface Mark {
  line: number;
  message: string;
}

interface Session {
  view: EditorView;
  language: Compartment;
  theme: Compartment;
  changeTimer: number | undefined;
}

const CHANGE_DEBOUNCE_MS = 150;

const sessions = new Map<string, Session>();
const requests = new Map<string, number>();

/** Marks a transaction made by Rust (create, setValue): it is not echoed back as a change. */
const fromRust = Annotation.define<boolean>();

function hostId(id: string): string {
  return `berth-editor-${id}`;
}

// --- Dockerfile: a small stream tokenizer (no legacy-modes package) ---

const INSTRUCTIONS = new Set([
  "FROM", "RUN", "CMD", "LABEL", "MAINTAINER", "EXPOSE", "ENV", "ADD", "COPY", "ENTRYPOINT",
  "VOLUME", "USER", "WORKDIR", "ARG", "ONBUILD", "STOPSIGNAL", "HEALTHCHECK", "SHELL",
]);

interface DockerState {
  instructionNext: boolean;
  continued: boolean; // the previous line ended with a backslash
}

const dockerfile = StreamLanguage.define<DockerState>({
  name: "dockerfile",
  startState: () => ({ instructionNext: true, continued: false }),
  copyState: (state) => ({ ...state }),
  token(stream, state) {
    if (stream.sol()) {
      state.instructionNext = !state.continued;
      state.continued = /\\\s*$/.test(stream.string);
      stream.eatSpace();
      if (stream.peek() === "#") {
        stream.skipToEnd();
        return "comment";
      }
    } else if (stream.eatSpace()) {
      return null;
    }
    const quote = stream.peek();
    if (quote === '"' || quote === "'") {
      stream.next();
      let escaped = false;
      let ch: string | void;
      while ((ch = stream.next()) !== undefined) {
        if (ch === quote && !escaped) break;
        escaped = !escaped && ch === "\\";
      }
      return "string";
    }
    if (stream.match(/^\$(\{[^}]*\}|[A-Za-z_][A-Za-z0-9_]*)/)) return "variableName";
    if (stream.match(/^[A-Za-z][A-Za-z0-9_-]*/)) {
      const word = stream.current().toUpperCase();
      if (state.instructionNext && INSTRUCTIONS.has(word)) {
        state.instructionNext = false;
        return "keyword";
      }
      if (word === "AS") return "keyword";
      state.instructionNext = false;
      return null;
    }
    if (stream.match(/^\d+/)) return "number";
    stream.next();
    return null;
  },
  blankLine(state) {
    state.continued = false;
  },
  languageData: { commentTokens: { line: "#" } },
});

function languageFor(lang: Lang): Extension {
  switch (lang) {
    case "yaml":
      return yaml();
    case "dockerfile":
      return dockerfile;
    case "text":
      return [];
  }
}

// --- Themes from the tokens ---

const highlight = HighlightStyle.define([
  { tag: [tags.definition(tags.propertyName), tags.propertyName], color: "var(--color-accent)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--color-success)" },
  { tag: [tags.keyword, tags.operatorKeyword], color: "var(--color-info)", fontWeight: "500" },
  { tag: [tags.number, tags.bool, tags.atom, tags.null], color: "var(--color-warning)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--color-ink-muted)" },
  { tag: [tags.labelName, tags.variableName, tags.typeName], color: "var(--color-series-3)" },
  { tag: [tags.meta, tags.separator, tags.punctuation], color: "var(--color-ink-muted)" },
  { tag: tags.invalid, color: "var(--color-danger)" },
]);

function themeExtension(theme: ThemeName): Extension {
  return EditorView.theme(
    {
      "&": {
        height: "100%",
        color: "var(--color-ink)",
        backgroundColor: "var(--color-surface-sunken)",
        fontSize: "var(--text-code-size)",
      },
      "&.cm-focused": { outline: "2px solid var(--color-focus)", outlineOffset: "-2px" },
      ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "var(--text-code-line)" },
      ".cm-content": { caretColor: "var(--color-ink)" },
      ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--color-ink)" },
      ".cm-gutters": {
        backgroundColor: "var(--color-surface-sunken)",
        color: "var(--color-ink-muted)",
        border: "none",
      },
      ".cm-activeLine": { backgroundColor: "var(--color-surface-hover)" },
      ".cm-activeLineGutter": { backgroundColor: "var(--color-surface-hover)" },
      "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection":
        { backgroundColor: "var(--color-selection)" },
      ".cm-matchingBracket": { backgroundColor: "var(--color-selection)", outline: "none" },
      ".cm-tooltip": {
        color: "var(--color-ink)",
        backgroundColor: "var(--color-surface-raised)",
        border: "1px solid var(--color-line)",
        borderRadius: "var(--radius-md)",
        boxShadow: "var(--shadow-popover)",
      },
      ".cm-diagnostic-error": { borderLeftColor: "var(--color-danger-dot)" },
      ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy var(--color-danger-dot)" },
    },
    { dark: theme === "dark" },
  );
}

/** The colours follow the tokens by themselves; only the dark flag of the base theme needs a push. */
function setMode(theme: ThemeName): void {
  for (const session of sessions.values()) {
    session.view.dispatch({ effects: session.theme.reconfigure(themeExtension(theme)) });
  }
}

onThemeChange(() => setMode(isDark() ? "dark" : "light"));

// --- Messages ---

function flushChange(id: string, session: Session): void {
  session.changeTimer = undefined;
  emit({ type: "editor.change", id, text: session.view.state.doc.toString() });
}

function mount(id: string, element: HTMLElement, options: CreateOptions): void {
  const language = new Compartment();
  const theme = new Compartment();
  let session: Session | undefined;

  const view = new EditorView({
    parent: element,
    state: EditorState.create({
      doc: options.value,
      extensions: [
        basicSetup,
        syntaxHighlighting(highlight),
        lintGutter(),
        language.of(languageFor(options.lang)),
        theme.of(themeExtension(options.theme)),
        EditorState.readOnly.of(options.readOnly),
        EditorView.editable.of(!options.readOnly),
        keymap.of([
          {
            key: "Mod-s",
            preventDefault: true,
            run: () => {
              emit({ type: "editor.save", id });
              return true;
            },
          },
        ]),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged || !session) return;
          if (update.transactions.some((tr) => tr.annotation(fromRust))) return;
          window.clearTimeout(session.changeTimer);
          const current = session;
          session.changeTimer = window.setTimeout(() => flushChange(id, current), CHANGE_DEBOUNCE_MS);
        }),
      ],
    }),
  });
  session = { view, language, theme, changeTimer: undefined };
  sessions.set(id, session);
  emit({ type: "editor.ready", id });
}

function dispose(id: string): void {
  requests.set(id, (requests.get(id) ?? 0) + 1);
  const session = sessions.get(id);
  if (!session) return;
  sessions.delete(id);
  window.clearTimeout(session.changeTimer);
  session.view.destroy();
}

const api = {
  create(id: string, options: CreateOptions): void {
    dispose(id);
    const request = requests.get(id) ?? 0;
    whenElement(hostId(id), (element) => {
      if (requests.get(id) === request) mount(id, element, options);
    });
  },

  setValue(id: string, text: string): void {
    const session = sessions.get(id);
    if (!session) return;
    window.clearTimeout(session.changeTimer);
    session.changeTimer = undefined;
    const { view } = session;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      annotations: fromRust.of(true),
    });
  },

  getValue(id: string): void {
    const session = sessions.get(id);
    if (session) emit({ type: "editor.value", id, text: session.view.state.doc.toString() });
  },

  setMarks(id: string, marks: Mark[]): void {
    const session = sessions.get(id);
    if (!session) return;
    const { doc } = session.view.state;
    const diagnostics: Diagnostic[] = marks
      .filter((mark) => mark.line >= 1 && mark.line <= doc.lines)
      .map((mark) => {
        const line = doc.line(mark.line);
        return { from: line.from, to: line.to, severity: "error", message: mark.message };
      });
    session.view.dispatch(setDiagnostics(session.view.state, diagnostics));
  },

  setTheme(theme: ThemeName): void {
    setMode(theme);
  },

  dispose,
};

// The functions Rust may call (PLAN 7.5); bridge/tests.rs reads this list from the bundle.
sealApi("editor", api, ["create", "setValue", "getValue", "setMarks", "setTheme", "dispose"]);
