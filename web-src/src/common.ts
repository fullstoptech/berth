// Shared by both bundles. Each bundle is a classic script that adds its
// namespace to window.berth; whichever loads first creates the object, and
// the Rust boot script (crates/berth-app/src/bridge/mod.rs) installs
// window.berth.emit and window.berth.ui. Messages follow PLAN section 7.5 and
// nothing else.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface Berth {
  emit?: (message: { [key: string]: Json }) => void;
  ui?: {
    scrollTo(elementId: string, top: number): void;
    copy(text: string): void;
  };
  term?: unknown;
  editor?: unknown;
}

declare global {
  interface Window {
    berth: Berth;
  }
}

export const berth: Berth = (window.berth ??= {});

/** JS to Rust: one JSON message per call; ignored until Rust installs emit. */
export function emit(message: { [key: string]: Json }): void {
  berth.emit?.(message);
}

/** The mode the page is drawn in: the attribute Rust sets on <html>, else the OS. */
export function isDark(): boolean {
  const attribute = document.documentElement.dataset.theme;
  if (attribute) return attribute === "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

/**
 * Run `apply` whenever the theme changes. Rust sets data-theme on <html> from an
 * effect, which can land after a create call, so the bundles follow the page
 * instead of trusting the order of messages.
 */
export function onThemeChange(apply: () => void): void {
  new MutationObserver(apply).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", apply);
}

/**
 * Publish a namespace on window.berth. `calls` lists the functions Rust calls;
 * a missing one is reported at load time instead of when Rust first uses it.
 */
export function sealApi(
  namespace: "term" | "editor",
  api: { [name: string]: unknown },
  calls: string[],
): void {
  for (const name of calls) {
    if (typeof api[name] !== "function") console.error(`berth.${namespace}.${name} is missing`);
  }
  berth[namespace] = api;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(binary);
}

export function base64ToBytes(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/**
 * The host element of a view is rendered by Rust a moment before or after the
 * create call; wait for it for up to two seconds of animation frames.
 */
export function whenElement(elementId: string, found: (element: HTMLElement) => void): void {
  let frames = 0;
  const look = (): void => {
    const element = document.getElementById(elementId);
    if (element) {
      found(element);
    } else if (frames < 120) {
      frames += 1;
      requestAnimationFrame(look);
    }
  };
  look();
}

/** Computed value of a design token on <html>, whichever theme is active. */
export function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
