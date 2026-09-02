// Typed wrappers around the Tauri IPC commands exposed by the Rust backend.
// The UI only ever talks to the file through these functions (see the
// architecture principle: the UI does not know how bytes are read from disk).
import { invoke } from "@tauri-apps/api/core";

export interface FileInfo {
  path: string;
  name: string;
  len: number;
}

interface ByteWindow {
  offset: number;
  len: number;
  base64: string;
}

export interface Interpretations {
  offset: number;
  u8: number | null;
  i8: number | null;
  u16_le: number | null;
  u16_be: number | null;
  u32_le: number | null;
  u32_be: number | null;
  u64_le: number | null;
  u64_be: number | null;
  i32_le: number | null;
  i32_be: number | null;
  f32_le: number | null;
  f32_be: number | null;
  f64_le: number | null;
  f64_be: number | null;
}

export function openFile(path: string): Promise<FileInfo> {
  return invoke<FileInfo>("open_file", { path });
}

export function getFileInfo(): Promise<FileInfo | null> {
  return invoke<FileInfo | null>("get_file_info");
}

export async function readRange(offset: number, length: number): Promise<Uint8Array> {
  const w = await invoke<ByteWindow>("read_range", { offset, length });
  return base64ToBytes(w.base64);
}

export function interpret(offset: number): Promise<Interpretations> {
  return invoke<Interpretations>("interpret", { offset });
}

// --- Schema runtime (Phases 3-5) -------------------------------------------

/** A decoded scalar value. Mirrors the Rust `schema_runtime::Value` enum,
 *  which serializes as `{ kind, value }` (unit variants omit `value`). */
export type Value =
  | { kind: "u"; value: number }
  | { kind: "i"; value: number }
  | { kind: "f"; value: number }
  | { kind: "bool"; value: boolean }
  | { kind: "char"; value: string }
  | { kind: "str"; value: string }
  | { kind: "bytes"; value: number[] }
  | { kind: "struct" }
  | { kind: "array" }
  | { kind: "enum"; value: { value: number; name: string | null } }
  | { kind: "bitfield" };

/** A node in the parsed structure tree. Mirrors `schema_runtime::FieldNode`. */
export interface FieldNode {
  name: string;
  type_name: string;
  value: Value;
  offset: number;
  size: number;
  description: string;
  children: FieldNode[];
}

/** A detected file format. Mirrors the backend `DetectionOut`. */
export interface Detection {
  format: string;
  extension: string;
  description: string;
  confidence: number;
  /** Whether a built-in signature or an installed plugin recognized it. */
  source?: "builtin" | "plugin";
}

/** Detect known formats from the open file's header (magic numbers). */
export function detectFormat(): Promise<Detection[]> {
  return invoke<Detection[]>("detect_format");
}

// --- Analysis heuristics (Phase 8) -----------------------------------------

/** A readable string found by the scanner. Mirrors `analysis::StringHit`. */
export interface StringHit {
  offset: number;
  len: number;
  encoding: "ascii" | "utf16_le";
  text: string;
}

/** A semantic guess about the bytes at an offset. Mirrors `analysis::Guess`. */
export interface Guess {
  label: string;
  detail: string;
}

/** Scan the start of the file for readable strings (>= minLen chars). */
export function findStrings(minLen: number): Promise<StringHit[]> {
  return invoke<StringHit[]>("find_strings", { minLen });
}

/** Ask what the bytes at `offset` could be (string, timestamp, UUID, …). */
export function analyzeAt(offset: number): Promise<Guess[]> {
  return invoke<Guess[]>("analyze_at", { offset });
}

/** Byte entropy across the whole file, as `buckets` values in [0,1]. */
export function entropy(buckets: number): Promise<number[]> {
  return invoke<number[]>("entropy", { buckets });
}

/** A ready-made schema for a recognized format. Mirrors `BuiltinSchema`. */
export interface BuiltinSchema {
  text: string;
  entry: string;
  endian: Endianness;
}

/** Fetch the built-in schema for a detected format, or null if none. */
export function builtinSchema(format: string): Promise<BuiltinSchema | null> {
  return invoke<BuiltinSchema | null>("builtin_schema", { format });
}

export type SearchKind = "hex" | "text" | "utf16" | "value";

/** Search the open file for a pattern; returns match offsets. For the `value`
 *  kind, pass the integer `width` in bytes (1/2/4/8) and the `endian`. */
export function search(
  kind: SearchKind,
  query: string,
  width?: number,
  endian?: Endianness,
): Promise<number[]> {
  return invoke<number[]>("search", { kind, query, width: width ?? null, endian: endian ?? null });
}

/** Write schema text to a file on disk. */
export function saveSchema(path: string, text: string): Promise<void> {
  return invoke<void>("save_schema", { path, text });
}

/** Read schema text from a file on disk. */
export function loadSchema(path: string): Promise<string> {
  return invoke<string>("load_schema", { path });
}

export type Endianness = "le" | "be";

/** Parse `schemaText` and execute it against the open file. `entry` may be
 *  empty to use the schema's first struct. */
export function parseSchema(
  schemaText: string,
  entry: string,
  endian: Endianness,
): Promise<FieldNode> {
  return invoke<FieldNode>("parse_schema", { schemaText, entry, endian });
}

// --- Editing (Phase 10) ----------------------------------------------------

/** Edit-buffer state. Mirrors the Rust `EditStatus`. */
export interface EditStatus {
  dirty: boolean;
  dirty_count: number;
  can_undo: boolean;
  can_redo: boolean;
  dirty_offsets: number[];
}

/** The `Value` tags the backend can encode back into bytes. */
export type EditableKind = "u" | "i" | "f" | "bool" | "char" | "str" | "bytes";

/** Whether a decoded value can be edited in place (scalars, not struct/array). */
export function isEditableKind(kind: Value["kind"]): kind is EditableKind {
  return kind === "u" || kind === "i" || kind === "f" || kind === "bool" ||
    kind === "char" || kind === "str" || kind === "bytes";
}

/** Current edit-buffer status (dirty flag, undo/redo, edited offsets). */
export function editStatus(): Promise<EditStatus> {
  return invoke<EditStatus>("edit_status");
}

/** Overwrite raw bytes at `offset` as one undoable edit. */
export function setBytes(offset: number, bytes: Uint8Array): Promise<EditStatus> {
  return invoke<EditStatus>("set_bytes", { offset, dataBase64: bytesToBase64(bytes) });
}

/** Encode a typed value and overwrite the field at `offset`. */
export function setFieldValue(
  offset: number,
  size: number,
  kind: EditableKind,
  endian: Endianness,
  value: string,
): Promise<EditStatus> {
  return invoke<EditStatus>("set_field_value", { offset, size, kind, endian, value });
}

export function undoEdit(): Promise<EditStatus> {
  return invoke<EditStatus>("undo_edit");
}

export function redoEdit(): Promise<EditStatus> {
  return invoke<EditStatus>("redo_edit");
}

export function revertEdits(): Promise<EditStatus> {
  return invoke<EditStatus>("revert_edits");
}

/** Save pending edits in place (backs up the original to `<path>.bak`). */
export function saveFile(): Promise<FileInfo> {
  return invoke<FileInfo>("save_file");
}

/** Save the edited bytes to a new path and switch to editing it. */
export function saveFileAs(path: string): Promise<FileInfo> {
  return invoke<FileInfo>("save_file_as", { path });
}

// --- Schema library & sharing (Phase 12) -----------------------------------

/** An entry in the schema library. Mirrors the Rust `SchemaEntry`. */
export interface SchemaEntry {
  id: string;
  name: string;
  entry: string;
  endian: Endianness;
  description: string;
  source: "builtin" | "user" | "plugin";
}

/** A schema loaded for the editor. Mirrors the Rust `LoadedSchema`. */
export interface LoadedSchema {
  text: string;
  name: string;
  entry: string;
  endian: Endianness;
  description: string;
}

/** List all available schemas: bundled ones plus the user's saved library. */
export function libraryList(): Promise<SchemaEntry[]> {
  return invoke<SchemaEntry[]>("library_list");
}

/** Load a schema from the library by id. */
export function libraryLoad(id: string): Promise<LoadedSchema> {
  return invoke<LoadedSchema>("library_load", { id });
}

/** Save the current schema into the user's library. */
export function libraryAdd(
  name: string,
  entry: string,
  endian: Endianness,
  description: string,
  text: string,
): Promise<SchemaEntry> {
  return invoke<SchemaEntry>("library_add", { name, entry, endian, description, text });
}

/** Remove a user schema from the library. */
export function libraryRemove(id: string): Promise<void> {
  return invoke<void>("library_remove", { id });
}

/** Write the current schema (with metadata) to a path, for sharing. */
export function exportSchema(
  path: string,
  name: string,
  entry: string,
  endian: Endianness,
  description: string,
  text: string,
): Promise<void> {
  return invoke<void>("export_schema", { path, name, entry, endian, description, text });
}

/** Read a shared schema file, returning its text and metadata. */
export function importSchema(path: string): Promise<LoadedSchema> {
  return invoke<LoadedSchema>("import_schema", { path });
}

/** Fields for exporting the current schema as a registry plugin pack. */
export interface PluginPack {
  path: string;
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  formatName: string;
  extension: string;
  entry: string;
  endian: Endianness;
  confidence: number;
  detectOffset: number;
  detectHex: string;
  schemaText: string;
}

/** Write the current schema as a registry-ready `plugin.toml`. Validates that
 *  the schema parses before writing, so the pack is guaranteed installable. */
export function exportPlugin(pack: PluginPack): Promise<void> {
  return invoke<void>("export_plugin", pack as unknown as Record<string, unknown>);
}

// --- Format plugins (plan §18, Phase A) ------------------------------------

/** One format a plugin contributes. Mirrors the Rust `PluginFormatInfo`. */
export interface PluginFormatInfo {
  name: string;
  extension: string;
  description: string;
  confidence: number;
  /** Whether this format auto-detects (has a magic-number rule). */
  detects: boolean;
}

/** An installed plugin. Mirrors the Rust `PluginInfo`. `error` is set for a
 *  plugin file that failed to parse. */
export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  enabled: boolean;
  file: string;
  formats: PluginFormatInfo[];
  error: string | null;
}

/** List installed format plugins. */
export function pluginList(): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugin_list");
}

/** Install a plugin from a `.toml` file on disk. Validates it first. */
export function pluginInstall(path: string): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugin_install", { path });
}

/** Remove an installed plugin by its file name (works for broken ones too). */
export function pluginRemove(file: string): Promise<void> {
  return invoke<void>("plugin_remove", { file });
}

/** Enable or disable an installed plugin. */
export function pluginSetEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke<void>("plugin_set_enabled", { id, enabled });
}

// --- Format registry (Phase 2 — browse & install shared packs) -------------

/** One format contributed by a registry entry. Mirrors Rust `RegistryFormat`. */
export interface RegistryFormat {
  name: string;
  extension: string;
  detects: boolean;
  confidence: number;
}

/** A pack listed in the registry's index.json. Mirrors Rust `RegistryEntry`. */
export interface RegistryEntry {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  category: string;
  tags: string[];
  formats: RegistryFormat[];
  /** Repo-relative path to the installable plugin.toml (used by install). */
  path: string;
}

/** The registry catalog. Mirrors Rust `RegistryCatalog`. */
export interface RegistryCatalog {
  version: number;
  count: number;
  formats: RegistryEntry[];
}

/** Fetch the online registry catalog (index.json). Requires network. */
export function registryCatalog(): Promise<RegistryCatalog> {
  return invoke<RegistryCatalog>("registry_catalog");
}

/** Download a registry pack by its index `path` and install it locally. */
export function registryInstall(path: string): Promise<PluginInfo> {
  return invoke<PluginInfo>("registry_install", { path });
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function bytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}
