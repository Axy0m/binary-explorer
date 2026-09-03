// Cross-window state sync for pop-out panels.
//
// The main window is the single source of truth. It broadcasts a small UI
// snapshot on every relevant change; pop-out panel windows render from it and
// fetch their own byte data via the (app-global) IPC commands. Panels send
// user actions (selection, schema edits) back up; the main window applies
// them and re-broadcasts. Heavy data (the parsed tree, byte pages) is never serialized
// over events — panels re-derive it from the snapshot.
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Endianness } from "./api";

export type PanelId = "tree" | "vinspect" | "hex" | "preview" | "schema";

export const PANEL_TITLES: Record<PanelId, string> = {
  tree: "Parse tree",
  vinspect: "Value inspector",
  hex: "Hex view",
  preview: "Data preview",
  schema: "Schema editor",
};

export function isPanelId(v: string | null): v is PanelId {
  return (
    v === "tree" || v === "vinspect" || v === "hex" || v === "preview" || v === "schema"
  );
}

/** The slice of UI state the main window shares with pop-out panels. */
export interface UiSnapshot {
  filePath: string | null;
  fileLen: number;
  selected: number | null;
  highlight: { start: number; end: number } | null;
  endian: Endianness;
  viewMode: "hex" | "text";
  schemaText: string;
  entry: string;
  /** Latest parse error, so the schema pop-out can show the same squiggle. */
  schemaError: string | null;
  editVersion: number;
}

/** An action a pop-out panel sends back to the main window. */
export type PanelAction =
  | { type: "select"; offset: number }
  /** The schema pop-out edited the DSL text. */
  | { type: "schema"; text: string }
  /** The schema pop-out edited the entry-struct name. */
  | { type: "entry"; value: string }
  /** The schema pop-out asked for a re-parse. */
  | { type: "parse" };

const SYNC = "ui:sync";
const REQUEST = "ui:request";
const ACTION = "ui:action";

export function broadcastSnapshot(s: UiSnapshot): void {
  void emit(SYNC, s);
}
export function onSnapshot(cb: (s: UiSnapshot) => void): Promise<UnlistenFn> {
  return listen<UiSnapshot>(SYNC, (e) => cb(e.payload));
}

/** A newly opened panel asks the main window to (re)send the current snapshot. */
export function requestSnapshot(): void {
  void emit(REQUEST);
}
export function onRequest(cb: () => void): Promise<UnlistenFn> {
  return listen(REQUEST, () => cb());
}

export function sendAction(a: PanelAction): void {
  void emit(ACTION, a);
}
export function onAction(cb: (a: PanelAction) => void): Promise<UnlistenFn> {
  return listen<PanelAction>(ACTION, (e) => cb(e.payload));
}
