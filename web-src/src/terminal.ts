// window.berth.term: xterm.js hosted in the webview (PLAN 7.5, D41, ADR 0004).
// Rust owns the session; this file holds only the screen. Output arrives as
// base64 chunks and is written once per animation frame.

import "@xterm/xterm/css/xterm.css";
import "./terminal.css";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import { base64ToBytes, berth, bytesToBase64, emit, onThemeChange, sealApi, token, whenElement } from "./common";

type ThemeName = "light" | "dark";

interface CreateOptions {
  fontSize: number;
  theme: ThemeName; // see setTheme: colours are read from the tokens
  scrollback: number;
}

interface Session {
  term: Terminal;
  fit: FitAddon;
  observer: ResizeObserver;
  queue: Uint8Array[];
  flushing: boolean;
  cols: number;
  rows: number;
}

const sessions = new Map<string, Session>();
// A create call waits for its host element; dispose or a second create for the
// same id must cancel it, so each id carries the latest request number.
const requests = new Map<string, number>();

function hostId(id: string): string {
  return `berth-term-${id}`;
}

/** Terminal colours come from the design tokens, so both themes follow tokens.css. */
function xtermTheme(): ITheme {
  const pick = (name: string): string | undefined => token(name) || undefined;
  return {
    background: pick("--color-surface-sunken"),
    foreground: pick("--color-ink"),
    cursor: pick("--color-ink"),
    cursorAccent: pick("--color-surface-sunken"),
    selectionBackground: pick("--color-selection"),
    selectionInactiveBackground: pick("--color-surface-hover"),
    black: pick("--color-surface-inverse"),
    red: pick("--color-danger-dot"),
    green: pick("--color-success-dot"),
    yellow: pick("--color-warning-dot"),
    blue: pick("--color-info-dot"),
    magenta: pick("--color-series-3"),
    cyan: pick("--color-series-5"),
    white: pick("--color-ink-muted"),
    brightBlack: pick("--color-line-strong"),
    brightRed: pick("--color-danger"),
    brightGreen: pick("--color-success"),
    brightYellow: pick("--color-warning"),
    brightBlue: pick("--color-info"),
    brightMagenta: pick("--color-series-4"),
    brightCyan: pick("--color-series-2"),
    brightWhite: pick("--color-ink"),
  };
}

function applyTheme(): void {
  const colours = xtermTheme();
  for (const session of sessions.values()) session.term.options.theme = colours;
}

onThemeChange(applyTheme);

function flush(id: string, session: Session): void {
  session.flushing = false;
  if (sessions.get(id) !== session || session.queue.length === 0) return;
  const total = session.queue.reduce((sum, chunk) => sum + chunk.length, 0);
  const joined = new Uint8Array(total);
  let offset = 0;
  for (const chunk of session.queue) {
    joined.set(chunk, offset);
    offset += chunk.length;
  }
  session.queue = [];
  session.term.write(joined);
}

function refit(id: string, session: Session): void {
  if (sessions.get(id) !== session) return;
  try {
    session.fit.fit();
  } catch {
    return; // host hidden (display: none): keep the last size
  }
  const { cols, rows } = session.term;
  if (cols !== session.cols || rows !== session.rows) {
    session.cols = cols;
    session.rows = rows;
    emit({ type: "term.resize", id, cols, rows });
  }
}

function mount(id: string, element: HTMLElement, options: CreateOptions): void {
  const term = new Terminal({
    fontSize: options.fontSize,
    fontFamily: token("--font-mono"),
    scrollback: options.scrollback,
    theme: xtermTheme(),
    cursorBlink: true,
    allowProposedApi: false,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(element);

  const session: Session = {
    term,
    fit,
    observer: new ResizeObserver(() => {
      requestAnimationFrame(() => refit(id, session));
    }),
    queue: [],
    flushing: false,
    cols: 0,
    rows: 0,
  };
  sessions.set(id, session);

  term.onData((data) => {
    emit({ type: "term.input", id, data: bytesToBase64(new TextEncoder().encode(data)) });
  });
  term.onBinary((data) => {
    const bytes = Uint8Array.from(data, (char) => char.charCodeAt(0));
    emit({ type: "term.input", id, data: bytesToBase64(bytes) });
  });
  term.onTitleChange((title) => emit({ type: "term.title", id, title }));

  session.observer.observe(element);
  // The first size goes out before "ready": Rust sizes the exec from it.
  refit(id, session);
  emit({ type: "term.ready", id });
}

function dispose(id: string): void {
  requests.set(id, (requests.get(id) ?? 0) + 1);
  const session = sessions.get(id);
  if (!session) return;
  sessions.delete(id);
  session.observer.disconnect();
  session.term.dispose();
}

const api = {
  create(id: string, options: CreateOptions): void {
    dispose(id);
    const request = requests.get(id) ?? 0;
    whenElement(hostId(id), (element) => {
      if (requests.get(id) === request) mount(id, element, options);
    });
  },

  write(id: string, base64: string): void {
    const session = sessions.get(id);
    if (!session) return;
    session.queue.push(base64ToBytes(base64));
    if (!session.flushing) {
      session.flushing = true;
      requestAnimationFrame(() => flush(id, session));
    }
  },

  clear(id: string): void {
    sessions.get(id)?.term.clear();
  },

  focus(id: string): void {
    sessions.get(id)?.term.focus();
  },

  copySelection(id: string): void {
    const text = sessions.get(id)?.term.getSelection();
    if (text) berth.ui?.copy(text);
  },

  // The colours come from the tokens, which follow the theme attribute Rust
  // sets on <html>; the argument only says which mode was asked for. The page
  // is also watched (below), so the order of the two does not matter.
  setTheme(_theme: ThemeName): void {
    applyTheme();
  },

  dispose,
};

// The functions Rust may call (PLAN 7.5); bridge/tests.rs reads this list from the bundle.
sealApi("term", api, ["create", "write", "clear", "focus", "copySelection", "setTheme", "dispose"]);
