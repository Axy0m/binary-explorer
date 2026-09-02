//! Tauri backend for Binary Explorer.
//!
//! This layer is a thin bridge: it owns the currently open file's
//! [`BinaryReader`] and exposes commands the React UI calls over IPC. All real
//! work lives in the `binary-reader` crate (and, later, the schema crates). The
//! UI never touches the filesystem directly — it goes through these commands.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use binary_reader::{BinaryReader, Endian};
use file_editing::{encode_value, EditBuffer, ValueKind};
use plugin_host::PluginManifest;
use schema_library::Metadata;
use schema_runtime::FieldNode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

mod licensing;

/// The currently open file, if any.
struct AppState {
    open: Mutex<Option<OpenFile>>,
}

struct OpenFile {
    path: String,
    reader: BinaryReader,
    /// Pending, unsaved overwrite edits layered on top of `reader` (Phase 10).
    edits: EditBuffer,
}

impl OpenFile {
    fn new(path: String, reader: BinaryReader) -> Self {
        let edits = EditBuffer::new(reader.len());
        Self { path, reader, edits }
    }
}

#[derive(Serialize, Clone)]
struct FileInfo {
    path: String,
    name: String,
    len: u64,
}

/// A window of bytes, returned base64-encoded to keep the IPC payload compact.
#[derive(Serialize)]
struct ByteWindow {
    offset: u64,
    len: u32,
    /// Base64 of the raw bytes in this window.
    base64: String,
}

/// Endian/type interpretations of the bytes at an offset (plan section 11).
#[derive(Serialize)]
struct Interpretations {
    offset: u64,
    u8: Option<u8>,
    i8: Option<i8>,
    u16_le: Option<u16>,
    u16_be: Option<u16>,
    u32_le: Option<u32>,
    u32_be: Option<u32>,
    u64_le: Option<u64>,
    u64_be: Option<u64>,
    i32_le: Option<i32>,
    i32_be: Option<i32>,
    f32_le: Option<f32>,
    f32_be: Option<f32>,
    f64_le: Option<f64>,
    f64_be: Option<f64>,
}

fn file_info(path: &str, reader: &BinaryReader) -> FileInfo {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    FileInfo {
        path: path.to_string(),
        name,
        len: reader.len() as u64,
    }
}

/// Open a file by path (memory-mapped) and make it the active file.
#[tauri::command]
fn open_file(path: String, state: State<AppState>) -> Result<FileInfo, String> {
    let reader = BinaryReader::open(&path).map_err(|e| e.to_string())?;
    let info = file_info(&path, &reader);
    *state.open.lock().unwrap() = Some(OpenFile::new(path, reader));
    Ok(info)
}

/// Info about the active file, or `null` if none is open.
#[tauri::command]
fn get_file_info(state: State<AppState>) -> Option<FileInfo> {
    state
        .open
        .lock()
        .unwrap()
        .as_ref()
        .map(|f| file_info(&f.path, &f.reader))
}

/// Read a window of raw bytes from the active file.
#[tauri::command]
fn read_range(offset: u64, length: u32, state: State<AppState>) -> Result<ByteWindow, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;

    let offset_usize = usize::try_from(offset).map_err(|_| "offset too large".to_string())?;
    // Clamp the requested length to what actually remains, so the viewer can ask
    // for a full window near EOF without erroring.
    let remaining = file.reader.len().saturating_sub(offset_usize);
    let len = (length as usize).min(remaining);

    let mut bytes = file
        .reader
        .read_bytes_at(offset_usize, len)
        .map_err(|e| e.to_string())?
        .to_vec();
    // Layer any unsaved edits on top so the hex view shows current state.
    file.edits.apply_window(&mut bytes, offset_usize);

    Ok(ByteWindow {
        offset,
        len: len as u32,
        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// Interpret the bytes at `offset` as every primitive type/endianness that fits.
#[tauri::command]
fn interpret(offset: u64, state: State<AppState>) -> Result<Interpretations, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    let o = usize::try_from(offset).map_err(|_| "offset too large".to_string())?;

    // Read the (at most) 8 bytes any interpretation needs, edits applied, and
    // interpret at offset 0 within that small edited window.
    let avail = file.reader.len().saturating_sub(o).min(8);
    let mut win = file
        .reader
        .read_bytes_at(o, avail)
        .map_err(|e| e.to_string())?
        .to_vec();
    file.edits.apply_window(&mut win, o);
    let r = BinaryReader::from_bytes(win);

    Ok(Interpretations {
        offset,
        u8: r.read_u8_at(0).ok(),
        i8: r.read_i8_at(0).ok(),
        u16_le: r.read_u16_le_at(0).ok(),
        u16_be: r.read_u16_be_at(0).ok(),
        u32_le: r.read_u32_le_at(0).ok(),
        u32_be: r.read_u32_be_at(0).ok(),
        u64_le: r.read_u64_le_at(0).ok(),
        u64_be: r.read_u64_be_at(0).ok(),
        i32_le: r.read_i32_le_at(0).ok(),
        i32_be: r.read_i32_be_at(0).ok(),
        f32_le: r.read_f32_le_at(0).ok(),
        f32_be: r.read_f32_be_at(0).ok(),
        f64_le: r.read_f64_le_at(0).ok(),
        f64_be: r.read_f64_be_at(0).ok(),
    })
}

/// A ready-made schema shipped with the app for a recognized format.
#[derive(Serialize)]
struct BuiltinSchema {
    text: String,
    entry: String,
    endian: String,
}

/// Return a ready-made schema for a detected format name, if one exists —
/// first the schemas embedded at compile time from the repo's `schemas/`
/// directory, then any enabled plugin that contributes that format.
#[tauri::command]
fn builtin_schema(app: AppHandle, format: String) -> Option<BuiltinSchema> {
    let embedded = match format.as_str() {
        "PNG" => Some((include_str!("../../../../schemas/png.schema"), "PNG", "be")),
        "BMP" => Some((include_str!("../../../../schemas/bmp.schema"), "BMP", "le")),
        "WAV" => Some((include_str!("../../../../schemas/wav.schema"), "WAV", "le")),
        "GZIP" => Some((include_str!("../../../../schemas/gzip.schema"), "Gzip", "le")),
        "ELF" => Some((include_str!("../../../../schemas/elf.schema"), "Elf64", "le")),
        "ZIP" => Some((include_str!("../../../../schemas/zip.schema"), "ZipLocalFileHeader", "le")),
        "PE" => Some((include_str!("../../../../schemas/pe.schema"), "PE", "le")),
        "SQLite" => Some((include_str!("../../../../schemas/sqlite.schema"), "SqliteHeader", "be")),
        _ => None,
    };
    if let Some((text, entry, endian)) = embedded {
        return Some(BuiltinSchema {
            text: text.to_string(),
            entry: entry.to_string(),
            endian: endian.to_string(),
        });
    }
    // Fall back to an enabled plugin that provides this format.
    let (plugins, _) = load_plugins(&app).ok()?;
    for p in plugins.iter().filter(|p| p.enabled) {
        if let Some(f) = p.manifest.format(&format) {
            return Some(BuiltinSchema {
                text: f.schema.clone(),
                entry: f.entry.clone(),
                endian: f.endian.clone(),
            });
        }
    }
    None
}

/// Cap on how many search matches to return, so a common pattern in a huge
/// file doesn't flood the UI.
const SEARCH_LIMIT: usize = 5000;

/// Search the active file for a pattern. `kind` is `"hex"`, `"text"`, or
/// `"utf16"`; `query` is interpreted accordingly. Returns match offsets.
#[tauri::command]
fn search(
    kind: String,
    query: String,
    width: Option<u8>,
    endian: Option<String>,
    state: State<AppState>,
) -> Result<Vec<u64>, String> {
    let needle = match kind.as_str() {
        "hex" => search::parse_hex(&query)?,
        "text" => search::text_bytes(&query),
        "utf16" => search::text_utf16le(&query),
        "value" => {
            let w = width.ok_or("value search needs a width")? as usize;
            let big_endian = matches!(endian.as_deref(), Some("be"));
            search::encode_int(&query, w, big_endian)?
        }
        other => return Err(format!("unknown search kind: {other}")),
    };
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    let hay = file.reader.read_bytes_at(0, file.reader.len()).map_err(|e| e.to_string())?;
    Ok(search::find_all(hay, &needle, SEARCH_LIMIT)
        .into_iter()
        .map(|o| o as u64)
        .collect())
}

/// Write schema text to a file on disk (the UI picks the path via a dialog).
#[tauri::command]
fn save_schema(path: String, text: String) -> Result<(), String> {
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Read schema text from a file on disk.
#[tauri::command]
fn load_schema(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Cap on how much of a large file the string scanner reads in one pass, and
/// how many hits it returns, so scanning a multi-GB file stays responsive.
const STRING_SCAN_BYTES: usize = 1 << 20; // 1 MiB
const STRING_SCAN_LIMIT: usize = 1000;

/// Scan the start of the active file for readable strings (plan §12).
#[tauri::command]
fn find_strings(min_len: usize, state: State<AppState>) -> Result<Vec<analysis::StringHit>, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    let scan_len = file.reader.len().min(STRING_SCAN_BYTES);
    let mut bytes = file.reader.read_bytes_at(0, scan_len).map_err(|e| e.to_string())?.to_vec();
    file.edits.apply_window(&mut bytes, 0);
    let mut hits = analysis::find_strings(&bytes, min_len.max(1));
    hits.truncate(STRING_SCAN_LIMIT);
    Ok(hits)
}

/// Byte-entropy across the file, as `buckets` normalized values in [0,1]
/// (plan §16 / design's "byte entropy" strip). High = random/compressed.
#[tauri::command]
fn entropy(buckets: usize, state: State<AppState>) -> Result<Vec<f32>, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    let base = file
        .reader
        .read_bytes_at(0, file.reader.len())
        .map_err(|e| e.to_string())?;
    let bytes = if file.edits.is_dirty() {
        file.edits.materialize(base)
    } else {
        base.to_vec()
    };
    Ok(analysis::entropy(&bytes, buckets.max(1)))
}

/// Guess what the bytes at `offset` could be — string, timestamp, UUID, etc.
#[tauri::command]
fn analyze_at(offset: u64, state: State<AppState>) -> Result<Vec<analysis::Guess>, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    let o = usize::try_from(offset).map_err(|_| "offset too large".to_string())?;
    // Read up to 16 bytes (the widest guess) from the offset.
    let avail = file.reader.len().saturating_sub(o).min(16);
    let mut bytes = file.reader.read_bytes_at(o, avail).map_err(|e| e.to_string())?.to_vec();
    file.edits.apply_window(&mut bytes, o);
    Ok(analysis::analyze_at(&bytes, 0))
}

/// A recognized format, from a built-in signature or a plugin.
#[derive(Serialize)]
struct DetectionOut {
    format: String,
    extension: String,
    description: String,
    confidence: u8,
    /// `"builtin"` or `"plugin"`.
    source: String,
}

/// Detect which known formats the active file's header matches (plan §13).
/// Combines the built-in signature registry with enabled format plugins.
/// Returns the candidates most-confident first, or an empty list if unknown.
#[tauri::command]
fn detect_format(app: AppHandle, state: State<AppState>) -> Result<Vec<DetectionOut>, String> {
    // Read the head, then drop the lock before touching the plugins directory.
    let head = {
        let guard = state.open.lock().unwrap();
        let file = guard.as_ref().ok_or("no file open")?;
        // The deepest signature ends near offset 262; 512 bytes is a safe head.
        let head_len = file.reader.len().min(512);
        file.reader
            .read_bytes_at(0, head_len)
            .map_err(|e| e.to_string())?
            .to_vec()
    };

    let mut out: Vec<DetectionOut> = format_detection::detect(&head)
        .into_iter()
        .map(|d| DetectionOut {
            format: d.format.to_string(),
            extension: d.extension.to_string(),
            description: d.description.to_string(),
            confidence: d.confidence,
            source: "builtin".to_string(),
        })
        .collect();

    // Enabled plugins may recognize additional formats.
    if let Ok((plugins, _)) = load_plugins(&app) {
        for p in plugins.iter().filter(|p| p.enabled) {
            for f in p.manifest.matches(&head) {
                out.push(DetectionOut {
                    format: f.name.clone(),
                    extension: f.extension.clone(),
                    description: f.description.clone(),
                    confidence: f.confidence,
                    source: "plugin".to_string(),
                });
            }
        }
    }

    out.sort_by(|a, b| b.confidence.cmp(&a.confidence));
    Ok(out)
}

/// Parse `schema_text` as a schema and execute it against the active file,
/// returning the parsed structure tree (plan phases 3-5). `entry` names the
/// struct to start from; if empty, the first struct in the schema is used.
/// `endian` is "be" for big-endian, anything else for little-endian.
#[tauri::command]
fn parse_schema(
    schema_text: String,
    entry: String,
    endian: String,
    state: State<AppState>,
) -> Result<FieldNode, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;

    let schema = schema_parser::parse(&schema_text).map_err(|e| e.to_string())?;

    // Default the entry point to the first struct defined.
    let entry = if entry.trim().is_empty() {
        schema
            .structs
            .first()
            .map(|s| s.name.clone())
            .ok_or("schema defines no structs")?
    } else {
        entry
    };

    let endian = if endian.eq_ignore_ascii_case("be") {
        Endian::Big
    } else {
        Endian::Little
    };

    // Parse against current bytes: the edited view when there are pending edits,
    // otherwise the mapped file directly (no copy).
    let edited;
    let reader = if file.edits.is_dirty() {
        let base = file
            .reader
            .read_bytes_at(0, file.reader.len())
            .map_err(|e| e.to_string())?;
        edited = BinaryReader::from_bytes(file.edits.materialize(base));
        &edited
    } else {
        &file.reader
    };

    schema_runtime::parse(&schema, reader, &entry, endian).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Editing (plan §17 / Phase 10)
// ---------------------------------------------------------------------------

/// Snapshot of the edit buffer for the UI: whether there are unsaved changes,
/// how many bytes differ, undo/redo availability, and which offsets are dirty
/// (so the hex view can tint edited bytes).
#[derive(Serialize)]
struct EditStatus {
    dirty: bool,
    dirty_count: usize,
    can_undo: bool,
    can_redo: bool,
    /// Edited byte offsets (capped) so the UI can highlight them.
    dirty_offsets: Vec<u64>,
}

/// Cap on how many dirty offsets we ship to the UI for highlighting.
const DIRTY_OFFSET_LIMIT: usize = 100_000;

fn edit_status_of(file: &OpenFile) -> EditStatus {
    EditStatus {
        dirty: file.edits.is_dirty(),
        dirty_count: file.edits.dirty_count(),
        can_undo: file.edits.can_undo(),
        can_redo: file.edits.can_redo(),
        dirty_offsets: file
            .edits
            .dirty_offsets()
            .take(DIRTY_OFFSET_LIMIT)
            .map(|o| o as u64)
            .collect(),
    }
}

/// Current edit-buffer status for the active file.
#[tauri::command]
fn edit_status(state: State<AppState>) -> Result<EditStatus, String> {
    let guard = state.open.lock().unwrap();
    let file = guard.as_ref().ok_or("no file open")?;
    Ok(edit_status_of(file))
}

/// Overwrite raw bytes at `offset` (base64-encoded) as a single undoable edit.
#[tauri::command]
fn set_bytes(offset: u64, data_base64: String, state: State<AppState>) -> Result<EditStatus, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|e| e.to_string())?;
    let o = usize::try_from(offset).map_err(|_| "offset too large".to_string())?;
    let mut guard = state.open.lock().unwrap();
    let file = guard.as_mut().ok_or("no file open")?;
    file.edits.set_bytes(o, &bytes).map_err(|e| e.to_string())?;
    Ok(edit_status_of(file))
}

/// Encode a typed value (`kind` matches a runtime `Value` tag: u/i/f/bool/char/
/// str/bytes) into `size` bytes and overwrite the field at `offset`. This is the
/// "edit parsed value → encode → write to change set" path from the plan.
#[tauri::command]
fn set_field_value(
    offset: u64,
    size: usize,
    kind: String,
    endian: String,
    value: String,
    state: State<AppState>,
) -> Result<EditStatus, String> {
    let kind = ValueKind::from_tag(&kind).ok_or_else(|| format!("cannot edit a {kind} field"))?;
    let endian = if endian.eq_ignore_ascii_case("be") {
        Endian::Big
    } else {
        Endian::Little
    };
    let bytes = encode_value(kind, size, endian, &value)?;
    let o = usize::try_from(offset).map_err(|_| "offset too large".to_string())?;

    let mut guard = state.open.lock().unwrap();
    let file = guard.as_mut().ok_or("no file open")?;
    file.edits.set_bytes(o, &bytes).map_err(|e| e.to_string())?;
    Ok(edit_status_of(file))
}

/// Undo the most recent edit.
#[tauri::command]
fn undo_edit(state: State<AppState>) -> Result<EditStatus, String> {
    let mut guard = state.open.lock().unwrap();
    let file = guard.as_mut().ok_or("no file open")?;
    file.edits.undo();
    Ok(edit_status_of(file))
}

/// Redo the most recently undone edit.
#[tauri::command]
fn redo_edit(state: State<AppState>) -> Result<EditStatus, String> {
    let mut guard = state.open.lock().unwrap();
    let file = guard.as_mut().ok_or("no file open")?;
    file.edits.redo();
    Ok(edit_status_of(file))
}

/// Discard all pending edits, reverting to the on-disk file.
#[tauri::command]
fn revert_edits(state: State<AppState>) -> Result<EditStatus, String> {
    let mut guard = state.open.lock().unwrap();
    let file = guard.as_mut().ok_or("no file open")?;
    file.edits.clear();
    Ok(edit_status_of(file))
}

/// Write the edited bytes to `target`, then reopen that file as the active one
/// with a clean edit buffer. `backup` writes a `.bak` copy of an existing target
/// first. Passing the original path saves in place; a new path is "Save As".
///
/// The current file is memory-mapped, so before overwriting the *same* path we
/// must drop the map (release the OS handle) or the write would fail on Windows.
/// We therefore rebuild the whole `OpenFile` around the freshly written file.
fn save_to(state: &State<AppState>, target: String, backup: bool) -> Result<FileInfo, String> {
    let mut guard = state.open.lock().unwrap();
    let file = guard.take().ok_or("no file open")?;

    // Materialize the edited bytes while we still hold the base map.
    let materialize_result = (|| -> Result<Vec<u8>, String> {
        let base = file
            .reader
            .read_bytes_at(0, file.reader.len())
            .map_err(|e| e.to_string())?;
        Ok(file.edits.materialize(base))
    })();

    let bytes = match materialize_result {
        Ok(b) => b,
        Err(e) => {
            // Put the file back so the app stays usable, then report the error.
            *guard = Some(file);
            return Err(e);
        }
    };

    // Optional backup of an existing target before we overwrite it.
    if backup && Path::new(&target).exists() {
        if let Err(e) = std::fs::copy(&target, format!("{target}.bak")) {
            *guard = Some(file);
            return Err(format!("backup failed: {e}"));
        }
    }

    // Drop the old reader (releases the memory map) before writing.
    drop(file);

    if let Err(e) = std::fs::write(&target, &bytes) {
        return Err(format!("write failed: {e}"));
    }

    // Reopen the just-written file as the active document, edits now clean.
    let reader = BinaryReader::open(&target).map_err(|e| e.to_string())?;
    let info = file_info(&target, &reader);
    *guard = Some(OpenFile::new(target, reader));
    Ok(info)
}

/// Save pending edits in place, backing up the original to `<path>.bak` first.
#[tauri::command]
fn save_file(state: State<AppState>) -> Result<FileInfo, String> {
    let target = {
        let guard = state.open.lock().unwrap();
        guard.as_ref().ok_or("no file open")?.path.clone()
    };
    save_to(&state, target, true)
}

/// Save the edited bytes to a new path and switch to editing that file.
#[tauri::command]
fn save_file_as(path: String, state: State<AppState>) -> Result<FileInfo, String> {
    save_to(&state, path, false)
}

// ---------------------------------------------------------------------------
// Schema library & sharing (plan §19 / Phase 12)
// ---------------------------------------------------------------------------

/// Schemas shipped with the app. Each carries its own `// @…` metadata header.
const BUILTINS: &[(&str, &str)] = &[
    ("png", include_str!("../../../../schemas/png.schema")),
    ("bmp", include_str!("../../../../schemas/bmp.schema")),
    ("wav", include_str!("../../../../schemas/wav.schema")),
    ("gzip", include_str!("../../../../schemas/gzip.schema")),
    ("elf", include_str!("../../../../schemas/elf.schema")),
    ("zip", include_str!("../../../../schemas/zip.schema")),
    ("pe", include_str!("../../../../schemas/pe.schema")),
    ("sqlite", include_str!("../../../../schemas/sqlite.schema")),
];

/// One entry in the schema library, as listed for the UI.
#[derive(Serialize)]
struct SchemaEntry {
    /// Stable id: `"builtin:<key>"` or `"user:<filename>"`.
    id: String,
    name: String,
    entry: String,
    endian: String,
    description: String,
    /// `"builtin"` (read-only) or `"user"` (deletable).
    source: String,
}

/// A schema loaded for the editor: its text plus resolved metadata.
#[derive(Serialize)]
struct LoadedSchema {
    text: String,
    name: String,
    entry: String,
    endian: String,
    description: String,
}

fn entry_from_meta(id: String, source: &str, meta: &Metadata, fallback_name: &str) -> SchemaEntry {
    let name = if meta.name.is_empty() {
        fallback_name.to_string()
    } else {
        meta.name.clone()
    };
    SchemaEntry {
        id,
        name,
        entry: meta.entry.clone(),
        endian: meta.endian.clone(),
        description: meta.description.clone(),
        source: source.to_string(),
    }
}

/// The per-user library directory (`<app-data>/library`), created if missing.
fn library_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let dir = base.join("library");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Turn a display name into a safe `.schema` filename.
fn slugify(name: &str) -> String {
    let mut slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() { "schema".to_string() } else { slug };
    format!("{base}.schema")
}

/// List all schemas available to load: bundled ones plus the user's library.
#[tauri::command]
fn library_list(app: AppHandle) -> Result<Vec<SchemaEntry>, String> {
    let mut out: Vec<SchemaEntry> = BUILTINS
        .iter()
        .map(|(key, text)| {
            let meta = schema_library::parse_metadata(text);
            entry_from_meta(format!("builtin:{key}"), "builtin", &meta, key)
        })
        .collect();

    // User schemas: every *.schema file in the library directory.
    let dir = library_dir(&app)?;
    let mut user: Vec<SchemaEntry> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for item in read.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some("schema") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let meta = schema_library::parse_metadata(&text);
            user.push(entry_from_meta(format!("user:{file}"), "user", &meta, &stem));
        }
    }
    user.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.extend(user);

    // Plugin-provided schemas (read-only, like built-ins).
    if let Ok((plugins, _)) = load_plugins(&app) {
        for p in plugins.iter().filter(|p| p.enabled) {
            for f in &p.manifest.formats {
                let meta = Metadata {
                    name: format!("{} ({})", f.name, p.manifest.name),
                    entry: f.entry.clone(),
                    endian: f.endian.clone(),
                    description: f.description.clone(),
                };
                out.push(entry_from_meta(
                    format!("plugin:{}:{}", p.manifest.id, f.name),
                    "plugin",
                    &meta,
                    &f.name,
                ));
            }
        }
    }
    Ok(out)
}

/// Load a schema (by its library id) into an editable form.
#[tauri::command]
fn library_load(app: AppHandle, id: String) -> Result<LoadedSchema, String> {
    // Plugin schemas take their metadata straight from the manifest (their DSL
    // text may not carry a `// @…` header), so resolve them up front.
    if let Some(rest) = id.strip_prefix("plugin:") {
        let (pid, fname) = rest
            .split_once(':')
            .ok_or("malformed plugin schema id")?;
        let (plugins, _) = load_plugins(&app)?;
        let plugin = plugins
            .iter()
            .find(|p| p.manifest.id == pid)
            .ok_or_else(|| format!("no plugin `{pid}`"))?;
        let fmt = plugin
            .manifest
            .format(fname)
            .ok_or_else(|| format!("plugin `{pid}` has no format `{fname}`"))?;
        return Ok(LoadedSchema {
            name: fmt.name.clone(),
            entry: fmt.entry.clone(),
            endian: fmt.endian.clone(),
            description: fmt.description.clone(),
            text: fmt.schema.clone(),
        });
    }

    let (text, fallback) = if let Some(key) = id.strip_prefix("builtin:") {
        let text = BUILTINS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, t)| t.to_string())
            .ok_or_else(|| format!("no built-in schema `{key}`"))?;
        (text, key.to_string())
    } else if let Some(file) = id.strip_prefix("user:") {
        let path = library_dir(&app)?.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let stem = Path::new(file).file_stem().unwrap_or_default().to_string_lossy().into_owned();
        (text, stem)
    } else {
        return Err(format!("unknown schema id `{id}`"));
    };

    let meta = schema_library::parse_metadata(&text);
    Ok(LoadedSchema {
        name: if meta.name.is_empty() { fallback } else { meta.name.clone() },
        entry: meta.entry.clone(),
        endian: meta.endian.clone(),
        description: meta.description.clone(),
        text,
    })
}

/// Save the current schema into the user's library (a `.schema` file with a
/// metadata header). Returns the new entry. Re-saving the same name overwrites.
#[tauri::command]
fn library_add(
    app: AppHandle,
    name: String,
    entry: String,
    endian: String,
    description: String,
    text: String,
) -> Result<SchemaEntry, String> {
    let meta = Metadata {
        name: name.clone(),
        entry,
        endian,
        description,
    };
    let body = schema_library::with_header(&meta, &text);
    let file = slugify(&name);
    let path = library_dir(&app)?.join(&file);
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(entry_from_meta(format!("user:{file}"), "user", &meta, &name))
}

/// Remove a user schema from the library. Built-in schemas cannot be removed.
#[tauri::command]
fn library_remove(app: AppHandle, id: String) -> Result<(), String> {
    let file = id
        .strip_prefix("user:")
        .ok_or("only user schemas can be removed")?;
    // Guard against path traversal: keep to a bare file name in the library dir.
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return Err("invalid schema id".to_string());
    }
    let path = library_dir(&app)?.join(file);
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

/// Write a schema (with a metadata header) to an arbitrary path, for sharing.
#[tauri::command]
fn export_schema(
    path: String,
    name: String,
    entry: String,
    endian: String,
    description: String,
    text: String,
) -> Result<(), String> {
    let meta = Metadata { name, entry, endian, description };
    std::fs::write(&path, schema_library::with_header(&meta, &text)).map_err(|e| e.to_string())
}

/// Read a shared schema file, returning its text and resolved metadata.
#[tauri::command]
fn import_schema(path: String) -> Result<LoadedSchema, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let meta = schema_library::parse_metadata(&text);
    let fallback = Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(LoadedSchema {
        name: if meta.name.is_empty() { fallback } else { meta.name.clone() },
        entry: meta.entry.clone(),
        endian: meta.endian.clone(),
        description: meta.description.clone(),
        text,
    })
}

/// Serialize a string as a TOML basic string (quoted, with escapes).
fn toml_basic(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Export the current schema as a registry-ready plugin pack (`plugin.toml`).
/// This turns an app-authored schema into the exact declarative file the
/// registry accepts: metadata + one format with a magic-number detect rule and
/// the inline schema. The schema is validated (it must parse) before writing,
/// so the exported pack is guaranteed installable.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn export_plugin(
    path: String,
    id: String,
    name: String,
    version: String,
    author: String,
    description: String,
    format_name: String,
    extension: String,
    entry: String,
    endian: String,
    confidence: u8,
    detect_offset: u64,
    detect_hex: String,
    schema_text: String,
) -> Result<(), String> {
    // Same grammar check plugin install runs — refuse to export a broken pack.
    schema_parser::parse(&schema_text).map_err(|e| format!("schema does not parse: {e}"))?;

    let id = id.trim();
    if id.is_empty() {
        return Err("a format id is required".into());
    }
    let slug_ok = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id.chars().next().is_some_and(|c| c != '-');
    if !slug_ok {
        return Err("id must be a lowercase slug (letters, digits, hyphens)".into());
    }
    if format_name.trim().is_empty() {
        return Err("a format name is required".into());
    }
    if confidence > 100 {
        return Err("confidence must be 0..=100".into());
    }
    // A literal multiline TOML string cannot contain a run of three quotes.
    if schema_text.contains("'''") {
        return Err("schema text cannot contain ''' ".into());
    }
    let endian = if endian == "be" { "be" } else { "le" };

    let hex = detect_hex.trim();
    if !hex.is_empty() {
        let cleaned: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        if cleaned.is_empty()
            || cleaned.len() % 2 != 0
            || !cleaned.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err("detect hex must be pairs of hex digits (e.g. FF D8 FF)".into());
        }
    }

    let mut out = String::new();
    out.push_str(&format!("id = {}\n", toml_basic(id)));
    out.push_str(&format!("name = {}\n", toml_basic(&name)));
    let version = if version.trim().is_empty() { "1.0.0" } else { version.trim() };
    out.push_str(&format!("version = {}\n", toml_basic(version)));
    if !description.trim().is_empty() {
        out.push_str(&format!("description = {}\n", toml_basic(description.trim())));
    }
    if !author.trim().is_empty() {
        out.push_str(&format!("author = {}\n", toml_basic(author.trim())));
    }

    out.push_str("\n[[formats]]\n");
    out.push_str(&format!("name = {}\n", toml_basic(format_name.trim())));
    if !extension.trim().is_empty() {
        out.push_str(&format!("extension = {}\n", toml_basic(extension.trim())));
    }
    if !description.trim().is_empty() {
        out.push_str(&format!("description = {}\n", toml_basic(description.trim())));
    }
    out.push_str(&format!("confidence = {confidence}\n"));
    if !entry.trim().is_empty() {
        out.push_str(&format!("entry = {}\n", toml_basic(entry.trim())));
    }
    out.push_str(&format!("endian = \"{endian}\"\n"));
    if !hex.is_empty() {
        out.push_str(&format!(
            "detect = [{{ offset = {detect_offset}, hex = {} }}]\n",
            toml_basic(hex)
        ));
    }
    // The schema goes in a literal multiline string so nothing is re-escaped.
    out.push_str("schema = '''\n");
    out.push_str(&schema_text);
    if !schema_text.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("'''\n");

    std::fs::write(&path, out).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Format plugins (plan §18 — plugin architecture, Phase A: declarative plugins)
// ---------------------------------------------------------------------------

/// The per-user plugins directory (`<app-data>/plugins`), created if missing.
fn plugins_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let dir = base.join("plugins");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Ids of plugins the user has disabled, stored as a JSON array in the plugins
/// directory. A missing/unreadable file means nothing is disabled.
fn load_disabled(dir: &Path) -> HashSet<String> {
    std::fs::read_to_string(dir.join("disabled.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn save_disabled(dir: &Path, set: &HashSet<String>) -> Result<(), String> {
    let v: Vec<&String> = set.iter().collect();
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("disabled.json"), text).map_err(|e| e.to_string())
}

/// A plugin file that failed to parse, surfaced so the user can fix or remove it.
struct BadPlugin {
    file: String,
    error: String,
}

/// A successfully loaded plugin: its manifest, source file name, and on/off.
struct LoadedPlugin {
    manifest: PluginManifest,
    file: String,
    enabled: bool,
}

/// Read and parse every `*.toml` in the plugins directory (skipping the
/// `disabled.json` state file). Returns the valid plugins and the invalid ones.
fn load_plugins(app: &AppHandle) -> Result<(Vec<LoadedPlugin>, Vec<BadPlugin>), String> {
    let dir = plugins_dir(app)?;
    let disabled = load_disabled(&dir);
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for item in read.flatten() {
            let path = item.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match PluginManifest::parse(&text) {
                Ok(m) => {
                    let enabled = !disabled.contains(&m.id);
                    ok.push(LoadedPlugin { manifest: m, file, enabled });
                }
                Err(error) => bad.push(BadPlugin { file, error }),
            }
        }
    }
    ok.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    Ok((ok, bad))
}

/// A safe `<id>.toml` file name for a plugin id.
fn plugin_filename(id: &str) -> String {
    let mut slug: String = id
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() { "plugin".to_string() } else { slug };
    format!("{base}.toml")
}

/// One format a plugin contributes, as listed for the UI.
#[derive(Serialize)]
struct PluginFormatInfo {
    name: String,
    extension: String,
    description: String,
    confidence: u8,
    /// Whether this format has a detection rule (auto-detects on open).
    detects: bool,
}

/// A plugin as listed for the UI. `error` is set for files that failed to parse.
#[derive(Serialize)]
struct PluginInfo {
    id: String,
    name: String,
    version: String,
    description: String,
    author: String,
    enabled: bool,
    file: String,
    formats: Vec<PluginFormatInfo>,
    error: Option<String>,
}

fn plugin_info(p: &LoadedPlugin) -> PluginInfo {
    PluginInfo {
        id: p.manifest.id.clone(),
        name: p.manifest.name.clone(),
        version: p.manifest.version.clone(),
        description: p.manifest.description.clone(),
        author: p.manifest.author.clone(),
        enabled: p.enabled,
        file: p.file.clone(),
        formats: p
            .manifest
            .formats
            .iter()
            .map(|f| PluginFormatInfo {
                name: f.name.clone(),
                extension: f.extension.clone(),
                description: f.description.clone(),
                confidence: f.confidence,
                detects: !f.detect.is_empty(),
            })
            .collect(),
        error: None,
    }
}

/// List installed plugins (valid ones first, then any that failed to parse).
#[tauri::command]
fn plugin_list(app: AppHandle) -> Result<Vec<PluginInfo>, String> {
    let (plugins, bad) = load_plugins(&app)?;
    let mut out: Vec<PluginInfo> = plugins.iter().map(plugin_info).collect();
    for b in bad {
        out.push(PluginInfo {
            id: String::new(),
            name: b.file.clone(),
            version: String::new(),
            description: String::new(),
            author: String::new(),
            enabled: false,
            file: b.file,
            formats: Vec::new(),
            error: Some(b.error),
        });
    }
    Ok(out)
}

/// Install a plugin from a `.toml` file the user picked. Validates the manifest
/// and that every embedded schema parses, then copies it into the plugins dir
/// (keyed by id, so re-installing upgrades in place). Returns the new entry.
#[tauri::command]
fn plugin_install(app: AppHandle, path: String) -> Result<PluginInfo, String> {
    // Licensing seam (no-op today — Free allows Plugins). If the plugin tier
    // becomes paid, this is where it enforces without touching the rest.
    licensing::require(licensing::Feature::Plugins)?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    install_plugin_text(&app, &text)
}

/// Validate plugin TOML (manifest + embedded schemas) and write it into the
/// plugins directory. Shared by installing from a local file and from the
/// registry — both must pass the exact same checks before landing on disk.
fn install_plugin_text(app: &AppHandle, text: &str) -> Result<PluginInfo, String> {
    let manifest = PluginManifest::parse(text)?;
    manifest.validate()?;
    // The app owns the schema parser, so validate the embedded schemas here.
    for f in &manifest.formats {
        schema_parser::parse(&f.schema)
            .map_err(|e| format!("format `{}` has an invalid schema: {e}", f.name))?;
    }
    let dir = plugins_dir(app)?;
    let file = plugin_filename(&manifest.id);
    std::fs::write(dir.join(&file), text).map_err(|e| e.to_string())?;
    // Re-installing clears any prior disabled state for this id.
    let mut disabled = load_disabled(&dir);
    if disabled.remove(&manifest.id) {
        let _ = save_disabled(&dir, &disabled);
    }
    Ok(plugin_info(&LoadedPlugin { manifest, file, enabled: true }))
}

/// Remove an installed plugin by its file name (works for valid plugins and
/// broken plugin files alike, since both have a file name).
#[tauri::command]
fn plugin_remove(app: AppHandle, file: String) -> Result<(), String> {
    // Keep to a bare file name inside the plugins directory.
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return Err("invalid plugin file".to_string());
    }
    let dir = plugins_dir(&app)?;
    let path = dir.join(&file);
    // Best-effort: drop any disabled-state entry for this plugin's id.
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(m) = PluginManifest::parse(&text) {
            let mut disabled = load_disabled(&dir);
            if disabled.remove(&m.id) {
                let _ = save_disabled(&dir, &disabled);
            }
        }
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

/// Enable or disable an installed plugin (persisted across restarts).
#[tauri::command]
fn plugin_set_enabled(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let dir = plugins_dir(&app)?;
    let mut disabled = load_disabled(&dir);
    if enabled {
        disabled.remove(&id);
    } else {
        disabled.insert(id);
    }
    save_disabled(&dir, &disabled)
}

// ---------------------------------------------------------------------------
// Format registry (plan §18, Phase 2 — browse & install shared format packs)
// ---------------------------------------------------------------------------
//
// The registry is a public git repo of declarative format packs (the same
// `plugin.toml` files the local install accepts). Fetching happens here in the
// Rust backend — not the webview — so the app keeps its null CSP and the only
// outbound network path lives behind these two commands. Formats are inert data
// and are re-validated by `install_plugin_text` before landing on disk; no user
// file is ever uploaded (browsing/installing is download-only).

/// Base URL of the format registry (raw files on the `main` branch).
const REGISTRY_BASE: &str = "https://raw.githubusercontent.com/Majd42/nybble-registry/main";

/// One format contributed by a registry entry (mirrors the registry index).
#[derive(Serialize, Deserialize)]
struct RegistryFormat {
    #[serde(default)]
    name: String,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    detects: bool,
    #[serde(default)]
    confidence: u8,
}

/// A pack listed in the registry's `index.json`.
#[derive(Serialize, Deserialize)]
struct RegistryEntry {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    formats: Vec<RegistryFormat>,
    /// Repo-relative path to the installable `plugin.toml`.
    path: String,
}

/// The registry catalog (`index.json`).
#[derive(Serialize, Deserialize)]
struct RegistryCatalog {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    count: u32,
    #[serde(default)]
    formats: Vec<RegistryEntry>,
}

/// GET a URL as text, with a short timeout and a clear error on failure.
fn http_get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("could not reach the registry: {e}"))?
        .into_string()
        .map_err(|e| format!("registry returned unreadable data: {e}"))
}

/// Fetch and parse the registry catalog (`index.json`).
#[tauri::command]
fn registry_catalog() -> Result<RegistryCatalog, String> {
    let url = format!("{REGISTRY_BASE}/index.json");
    let text = http_get_text(&url)?;
    serde_json::from_str::<RegistryCatalog>(&text)
        .map_err(|e| format!("registry index is malformed: {e}"))
}

/// Download the plugin pack at `path` (from a catalog entry) and install it,
/// running the same validation as a local install. `path` is constrained to a
/// `formats/<id>/plugin.toml` shape so it can only pull packs from the registry.
#[tauri::command]
fn registry_install(app: AppHandle, path: String) -> Result<PluginInfo, String> {
    // Licensing seam (no-op today — Free allows Registry). The public registry
    // stays free; a future private/team registry would gate here.
    licensing::require(licensing::Feature::Registry)?;
    let ok = path.starts_with("formats/")
        && path.ends_with("/plugin.toml")
        && !path.contains("..")
        && !path.contains("//");
    if !ok {
        return Err(format!("refusing to fetch unexpected registry path: {path}"));
    }
    let url = format!("{REGISTRY_BASE}/{path}");
    let text = http_get_text(&url)?;
    install_plugin_text(&app, &text)
}

/// Report the current licensing tier and which capabilities it unlocks. The
/// UI can use this to show the tier (and, once paid features exist, reflect
/// what's available). Everything is `true` today.
#[tauri::command]
fn license_status() -> licensing::LicenseStatus {
    licensing::status()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(AppState {
                open: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_file,
            get_file_info,
            read_range,
            interpret,
            parse_schema,
            detect_format,
            find_strings,
            analyze_at,
            entropy,
            save_schema,
            load_schema,
            search,
            builtin_schema,
            edit_status,
            set_bytes,
            set_field_value,
            undo_edit,
            redo_edit,
            revert_edits,
            save_file,
            save_file_as,
            library_list,
            library_load,
            library_add,
            library_remove,
            export_schema,
            import_schema,
            export_plugin,
            plugin_list,
            plugin_install,
            plugin_remove,
            plugin_set_enabled,
            registry_catalog,
            registry_install,
            license_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running Binary Explorer");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_plugin_produces_installable_pack() {
        // A schema whose field docs contain double quotes and parentheses —
        // exactly the characters the literal-multiline embedding must survive.
        let schema = "struct Foo {\n    magic bytes[4] \"file magic (DEAD)\"\n    n     u32      \"a count\"\n}\n";
        let path = std::env::temp_dir().join("nybble_export_roundtrip.toml");
        export_plugin(
            path.to_string_lossy().into_owned(),
            "foo".into(),
            "Foo format".into(),
            "".into(),          // version -> defaults to 1.0.0
            "Tester".into(),
            "a test format".into(),
            "FOO".into(),
            "foo".into(),
            "Foo".into(),
            "le".into(),
            90,
            0,
            "DE AD BE EF".into(),
            schema.into(),
        )
        .expect("export should succeed");

        let text = std::fs::read_to_string(&path).unwrap();
        // The written file must parse and validate as a real plugin manifest,
        // and its embedded schema must still parse — i.e. it is installable.
        let m = PluginManifest::parse(&text).expect("exported TOML parses as a manifest");
        m.validate().expect("exported manifest validates");
        assert_eq!(m.id, "foo");
        assert_eq!(m.formats.len(), 1);
        schema_parser::parse(&m.formats[0].schema).expect("embedded schema parses");
        // The version defaulted, and the schema round-tripped byte-for-byte.
        assert_eq!(m.version, "1.0.0");
        assert!(m.formats[0].schema.contains("file magic (DEAD)"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn export_plugin_rejects_a_broken_schema() {
        let path = std::env::temp_dir().join("nybble_export_broken.toml");
        let err = export_plugin(
            path.to_string_lossy().into_owned(),
            "bad".into(),
            "Bad".into(),
            "1.0.0".into(),
            "".into(),
            "".into(),
            "BAD".into(),
            "".into(),
            "".into(),
            "le".into(),
            90,
            0,
            "".into(),
            "struct { this is not valid".into(),
        );
        assert!(err.is_err(), "a schema that doesn't parse must not export");
    }
}
