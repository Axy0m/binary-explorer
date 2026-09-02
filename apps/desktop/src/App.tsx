import { useEffect, useMemo, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  openFile,
  interpret,
  readRange,
  parseSchema,
  detectFormat,
  findStrings,
  analyzeAt,
  entropy,
  search,
  builtinSchema,
  setFieldValue,
  undoEdit,
  redoEdit,
  revertEdits,
  saveFile,
  saveFileAs,
  editStatus,
  isEditableKind,
  libraryList,
  libraryLoad,
  libraryAdd,
  libraryRemove,
  exportSchema,
  importSchema,
  type SchemaEntry,
  type SearchKind,
  type BuiltinSchema,
  type Detection,
  type Endianness,
  type EditStatus,
  type FieldNode,
  type FileInfo,
  type Interpretations,
  type StringHit,
  type Guess,
} from "./api";
import { HexView } from "./HexView";
import { FileMap } from "./FileMap";
import { SchemaEditor } from "./SchemaEditor";
import { Plugins } from "./Plugins";
import { ExportPack } from "./ExportPack";
import {
  broadcastSnapshot,
  onRequest,
  onAction,
  PANEL_TITLES,
  type PanelId,
  type UiSnapshot,
} from "./panelSync";
import { StructureTree, findFieldAtOffset } from "./StructureTree";
import { ValueInspector } from "./ValueInspector";
import { DataPreview } from "./DataPreview";
import { EntropyStrip } from "./EntropyStrip";
import { buildColorMap, colorAt as colorAtRange } from "./colors";

type Range = { start: number; end: number };

const SAMPLE_SCHEMA = `struct Header {
    magic   char[4]  "file magic"
    version u16      "schema revision"
    flags   u16      "bit flags"
    size    u32      "total size in bytes"
}`;

export function App() {
  const [file, setFile] = useState<FileInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [interp, setInterp] = useState<Interpretations | null>(null);
  const [gotoText, setGotoText] = useState("");
  const [formats, setFormats] = useState<Detection[]>([]);
  const [builtin, setBuiltin] = useState<BuiltinSchema | null>(null);
  const [entropyData, setEntropyData] = useState<number[]>([]);
  const [viewMode, setViewMode] = useState<"hex" | "text">("hex");
  const [strings, setStrings] = useState<StringHit[]>([]);
  const [guesses, setGuesses] = useState<Guess[]>([]);

  // Search
  const [searchKind, setSearchKind] = useState<SearchKind>("text");
  const [searchQuery, setSearchQuery] = useState("");
  const [valueWidth, setValueWidth] = useState(4); // bytes, for typed value search
  const [matches, setMatches] = useState<number[]>([]);
  const [matchIndex, setMatchIndex] = useState(0);
  const [matchLen, setMatchLen] = useState(0);

  // Schema / structure. Last-used schema is remembered across restarts.
  const [schemaText, setSchemaText] = useState(() => localStorage.getItem("schemaText") ?? SAMPLE_SCHEMA);
  const [entry, setEntry] = useState(() => localStorage.getItem("schemaEntry") ?? "");
  const [endian, setEndian] = useState<Endianness>(
    () => (localStorage.getItem("schemaEndian") as Endianness) ?? "le",
  );
  const [tree, setTree] = useState<FieldNode | null>(null);
  const [schemaError, setSchemaError] = useState<string | null>(null);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [highlight, setHighlight] = useState<Range | null>(null);
  const [selectedNode, setSelectedNode] = useState<FieldNode | null>(null);
  const [rawBytes, setRawBytes] = useState<Uint8Array | null>(null);

  // Schema library (Phase 12).
  const [library, setLibrary] = useState<SchemaEntry[]>([]);
  const [showPlugins, setShowPlugins] = useState(false);
  const [showPack, setShowPack] = useState(false);
  const [packNotice, setPackNotice] = useState<string | null>(null);
  // Resizable workspace columns (px). The hex column (3rd) is the flexible
  // filler; these three are drag-adjustable and persisted.
  const [colW, setColW] = useState<{ tree: number; vinspect: number; right: number }>(() => {
    const s = localStorage.getItem("colW");
    if (s) {
      try {
        const v = JSON.parse(s);
        if (v && typeof v.tree === "number") return v;
      } catch { /* fall through to defaults */ }
    }
    return { tree: 230, vinspect: 250, right: 340 };
  });
  useEffect(() => {
    localStorage.setItem("colW", JSON.stringify(colW));
  }, [colW]);
  const [savingLib, setSavingLib] = useState(false);
  const [libName, setLibName] = useState("");
  const [libDesc, setLibDesc] = useState("");

  // Editing (Phase 10). `editVersion` bumps after every edit so the hex view
  // re-fetches its byte pages; `edit` carries the dirty flag and edited offsets.
  const [edit, setEdit] = useState<EditStatus | null>(null);
  const [editVersion, setEditVersion] = useState(0);
  const dirtySet = useMemo(() => new Set(edit?.dirty_offsets ?? []), [edit]);
  const isEdited = (offset: number) => dirtySet.has(offset);

  // Field colors, shared by the hex view and the parse tree.
  const colorMap = useMemo(() => buildColorMap(tree), [tree]);
  const colorFor = (offset: number) => colorAtRange(colorMap, offset);

  // --- Pop-out panel sync. The main window is the source of truth: it
  // broadcasts a UI snapshot on change, answers a new panel's request for the
  // current one, and applies selection actions panels send back. ------------
  const snapRef = useRef<UiSnapshot | null>(null);
  useEffect(() => {
    const snap: UiSnapshot = {
      filePath: file?.path ?? null,
      fileLen: file?.len ?? 0,
      selected,
      highlight,
      endian,
      viewMode,
      schemaText,
      entry,
      editVersion,
    };
    snapRef.current = snap;
    broadcastSnapshot(snap);
  }, [file, selected, highlight, endian, viewMode, schemaText, entry, editVersion]);

  useEffect(() => {
    let alive = true;
    const uns: Array<() => void> = [];
    const track = (p: Promise<() => void>) =>
      p.then((u) => (alive ? uns.push(u) : u()));
    track(onRequest(() => snapRef.current && broadcastSnapshot(snapRef.current)));
    track(onAction((a) => a.type === "select" && selectByte(a.offset)));
    return () => {
      alive = false;
      uns.forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Open a panel in its own OS window (or focus it if already open). */
  async function popOut(panel: PanelId) {
    const label = `panel-${panel}`;
    try {
      const existing = await WebviewWindow.getByLabel(label);
      if (existing) {
        await existing.setFocus();
        return;
      }
    } catch {
      /* not open yet — create it */
    }
    const w = new WebviewWindow(label, {
      url: `index.html?panel=${panel}`,
      title: `Nybble — ${PANEL_TITLES[panel]}`,
      width: 480,
      height: 640,
    });
    w.once("tauri://error", (e) =>
      setError(`Could not open panel window: ${JSON.stringify(e.payload)}`),
    );
  }

  async function handleOpen() {
    setError(null);
    try {
      const path = await open({ multiple: false, directory: false });
      if (typeof path !== "string") return; // cancelled
      const info = await openFile(path);
      setFile(info);
      setSelected(null);
      setInterp(null);
      setTree(null);
      setActivePath(null);
      setHighlight(null);
      setSelectedNode(null);
      setRawBytes(null);
      setGuesses([]);
      setMatches([]);
      setMatchIndex(0);
      setEdit(null);
      setEditVersion((v) => v + 1);
      const detected = await detectFormat();
      setFormats(detected);
      setBuiltin(detected.length > 0 ? await builtinSchema(detected[0].format) : null);
      setStrings(await findStrings(4));
      setEntropyData(await entropy(256));
    } catch (e) {
      setError(String(e));
    }
  }

  // After plugins change, re-detect the open file and refresh the library so
  // any newly available formats/schemas show up immediately.
  async function handlePluginsChanged() {
    try {
      setLibrary(await libraryList());
    } catch {
      /* leave the current library on failure */
    }
    if (file) {
      try {
        const detected = await detectFormat();
        setFormats(detected);
        setBuiltin(detected.length > 0 ? await builtinSchema(detected[0].format) : null);
      } catch (e) {
        setError(String(e));
      }
    }
  }

  // Selected byte -> byte interpretations + semantic guesses.
  useEffect(() => {
    if (selected == null) {
      setInterp(null);
      setGuesses([]);
      return;
    }
    interpret(selected).then(setInterp).catch((e) => setError(String(e)));
    analyzeAt(selected).then(setGuesses).catch(() => setGuesses([]));
  }, [selected]);

  // Selected field -> fetch its raw bytes for the value inspector.
  useEffect(() => {
    if (!selectedNode || selectedNode.size === 0) {
      setRawBytes(null);
      return;
    }
    const n = Math.min(selectedNode.size, 32);
    readRange(selectedNode.offset, n).then(setRawBytes).catch(() => setRawBytes(null));
  }, [selectedNode]);

  // Persist schema settings.
  useEffect(() => {
    localStorage.setItem("schemaText", schemaText);
    localStorage.setItem("schemaEntry", entry);
    localStorage.setItem("schemaEndian", endian);
  }, [schemaText, entry, endian]);

  // Re-link a fresh tree to the current byte selection.
  useEffect(() => {
    if (tree == null || selected == null) return;
    const found = findFieldAtOffset(tree, selected);
    setActivePath(found?.path ?? null);
    setSelectedNode(found?.node ?? null);
    setHighlight(found ? { start: found.node.offset, end: found.node.offset + found.node.size } : null);
  }, [tree]);

  function handleGoto(e: React.FormEvent) {
    e.preventDefault();
    if (!file) return;
    const raw = gotoText.trim().replace(/^0x/i, "");
    const offset = parseInt(raw, 16);
    if (Number.isNaN(offset) || offset < 0 || offset >= file.len) {
      setError(`Offset out of range (0 .. 0x${(file.len - 1).toString(16)})`);
      return;
    }
    setError(null);
    selectByte(offset);
  }

  function needleLen(kind: SearchKind, query: string): number {
    if (kind === "hex") return query.replace(/\s/g, "").length / 2;
    if (kind === "utf16") return query.length * 2;
    if (kind === "value") return valueWidth;
    return new TextEncoder().encode(query).length;
  }

  async function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    if (!file || searchQuery === "") return;
    try {
      const hits =
        searchKind === "value"
          ? await search("value", searchQuery, valueWidth, endian)
          : await search(searchKind, searchQuery);
      setMatches(hits);
      setMatchIndex(0);
      const len = needleLen(searchKind, searchQuery);
      setMatchLen(len);
      if (hits.length > 0) jumpToMatch(hits[0], len);
      else setError(`No matches for ${searchKind} "${searchQuery}"`);
    } catch (err) {
      setError(String(err));
    }
  }

  function stepMatch(delta: number) {
    if (matches.length === 0) return;
    const next = (matchIndex + delta + matches.length) % matches.length;
    setMatchIndex(next);
    jumpToMatch(matches[next], matchLen);
  }

  function jumpToMatch(offset: number, len: number) {
    setSelected(offset);
    setHighlight({ start: offset, end: offset + len });
  }

  async function handleParse() {
    if (!file) return;
    setSchemaError(null);
    try {
      const root = await parseSchema(schemaText, entry, endian);
      setTree(root);
    } catch (e) {
      setTree(null);
      setActivePath(null);
      setHighlight(null);
      setSelectedNode(null);
      setSchemaError(String(e));
    }
  }

  async function handleUseBuiltin() {
    if (!builtin) return;
    setSchemaText(builtin.text);
    setEntry(builtin.entry);
    setEndian(builtin.endian);
    try {
      const root = await parseSchema(builtin.text, builtin.entry, builtin.endian);
      setTree(root);
      setSchemaError(null);
    } catch (e) {
      setTree(null);
      setSchemaError(String(e));
    }
  }

  // --- Schema library & sharing (Phase 12) ----------------------------------

  // Load the library (bundled + user schemas) once on startup.
  useEffect(() => {
    libraryList().then(setLibrary).catch(() => {});
  }, []);

  // Apply a loaded schema to the editor and parse it immediately.
  async function applyLoadedSchema(s: { text: string; entry: string; endian: Endianness }) {
    setSchemaText(s.text);
    setEntry(s.entry);
    setEndian(s.endian);
    try {
      setTree(await parseSchema(s.text, s.entry, s.endian));
      setSchemaError(null);
    } catch (e) {
      setTree(null);
      setSchemaError(String(e));
    }
  }

  async function handlePickFromLibrary(id: string) {
    if (!id) return;
    try {
      await applyLoadedSchema(await libraryLoad(id));
    } catch (e) {
      setSchemaError(String(e));
    }
  }

  function beginSaveToLibrary() {
    setLibName(entry || "My schema");
    setLibDesc("");
    setSavingLib(true);
  }

  async function confirmSaveToLibrary() {
    try {
      await libraryAdd(libName.trim() || "schema", entry, endian, libDesc.trim(), schemaText);
      setLibrary(await libraryList());
      setSavingLib(false);
    } catch (e) {
      setSchemaError(String(e));
    }
  }

  async function handleRemoveFromLibrary(id: string) {
    try {
      await libraryRemove(id);
      setLibrary(await libraryList());
    } catch (e) {
      setSchemaError(String(e));
    }
  }

  async function handleExportSchema() {
    try {
      const path = await save({
        title: "Export schema",
        defaultPath: `${(entry || "schema").toLowerCase()}.schema`,
        filters: [{ name: "Schema", extensions: ["schema"] }],
      });
      if (typeof path !== "string") return;
      await exportSchema(path, entry || "schema", entry, endian, "", schemaText);
    } catch (e) {
      setSchemaError(String(e));
    }
  }

  async function handleImportSchema() {
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Schema", extensions: ["schema", "txt"] }],
      });
      if (typeof path !== "string") return;
      await applyLoadedSchema(await importSchema(path));
    } catch (e) {
      setSchemaError(String(e));
    }
  }

  // Drag a column divider. The right column grows when dragged leftwards, so
  // its handle is inverted; the hex column absorbs the slack either way.
  function startResize(which: "tree" | "vinspect" | "right", e: React.PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = colW[which];
    const dir = which === "right" ? -1 : 1;
    function move(ev: PointerEvent) {
      const next = Math.max(140, Math.min(900, startW + dir * (ev.clientX - startX)));
      setColW((c) => ({ ...c, [which]: next }));
    }
    function up() {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    }
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }

  // Byte -> field.
  function selectByte(offset: number) {
    setSelected(offset);
    if (tree) {
      const found = findFieldAtOffset(tree, offset);
      setActivePath(found?.path ?? null);
      setSelectedNode(found?.node ?? null);
      setHighlight(found ? { start: found.node.offset, end: found.node.offset + found.node.size } : null);
    }
  }

  // Field -> bytes.
  function selectField(node: FieldNode, path: string) {
    setActivePath(path);
    setSelectedNode(node);
    setHighlight({ start: node.offset, end: node.offset + node.size });
    setSelected(node.offset);
  }

  // --- Editing (Phase 10) ---------------------------------------------------

  // After any edit, refresh the views that read bytes: bump the hex version,
  // re-run the schema so tree values update, and refresh the current selection.
  async function refreshAfterEdit(status: EditStatus) {
    setEdit(status);
    setEditVersion((v) => v + 1);
    if (tree) {
      try {
        setTree(await parseSchema(schemaText, entry, endian));
      } catch {
        /* keep the previous tree if a re-parse fails */
      }
    }
    if (selected != null) interpret(selected).then(setInterp).catch(() => {});
    if (selectedNode && selectedNode.size > 0) {
      readRange(selectedNode.offset, Math.min(selectedNode.size, 32)).then(setRawBytes).catch(() => {});
    }
  }

  // Commit an edited value for the selected field. Returns an error message to
  // show inline, or null on success.
  async function commitFieldEdit(value: string): Promise<string | null> {
    if (!selectedNode) return "No field selected.";
    const kind = selectedNode.value.kind;
    if (!isEditableKind(kind)) return "This field type can't be edited.";
    try {
      const status = await setFieldValue(selectedNode.offset, selectedNode.size, kind, endian, value);
      await refreshAfterEdit(status);
      return null;
    } catch (e) {
      return String(e);
    }
  }

  async function handleUndo() {
    try {
      await refreshAfterEdit(await undoEdit());
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRedo() {
    try {
      await refreshAfterEdit(await redoEdit());
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRevert() {
    try {
      await refreshAfterEdit(await revertEdits());
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSave() {
    try {
      const info = await saveFile();
      setFile(info);
      setEdit(await editStatus());
      setEditVersion((v) => v + 1);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSaveAs() {
    try {
      const path = await save({ title: "Save binary as", defaultPath: file?.name });
      if (typeof path !== "string") return;
      const info = await saveFileAs(path);
      setFile(info);
      setEdit(await editStatus());
      setEditVersion((v) => v + 1);
    } catch (e) {
      setError(String(e));
    }
  }

  // Editor keyboard shortcuts: Ctrl+S save, Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!file || !(e.ctrlKey || e.metaKey)) return;
      const k = e.key.toLowerCase();
      if (k === "s") { e.preventDefault(); handleSave(); }
      else if (k === "z" && !e.shiftKey) { e.preventDefault(); handleUndo(); }
      else if (k === "y" || (k === "z" && e.shiftKey)) { e.preventDefault(); handleRedo(); }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const valid = tree != null && !schemaError;
  const dirty = edit?.dirty ?? false;

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">Nybble</span>
        <button onClick={handleOpen}>Open File…</button>
        <button className="ghost" onClick={() => setShowPlugins(true)} title="Manage format plugins">Plugins</button>
        {file && <span className="tab">{file.name}</span>}

        {file && (
          <form className="goto" onSubmit={handleGoto}>
            <label>Go</label>
            <input value={gotoText} onChange={(e) => setGotoText(e.target.value)} placeholder="0x1A40" spellCheck={false} />
          </form>
        )}
        {file && (
          <form className="search" onSubmit={handleSearch}>
            <select value={searchKind} onChange={(e) => setSearchKind(e.target.value as SearchKind)}>
              <option value="text">Text</option>
              <option value="hex">Hex</option>
              <option value="utf16">UTF-16</option>
              <option value="value">Value</option>
            </select>
            {searchKind === "value" && (
              <select value={valueWidth} onChange={(e) => setValueWidth(Number(e.target.value))} title="Integer width (uses the schema's endianness)">
                <option value={1}>u8</option>
                <option value={2}>u16</option>
                <option value={4}>u32</option>
                <option value={8}>u64</option>
              </select>
            )}
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={searchKind === "hex" ? "89 50" : searchKind === "value" ? "42 or 0x2A" : "find…"}
              spellCheck={false}
            />
            <button type="submit">Find</button>
            {matches.length > 0 && (
              <>
                <button type="button" className="ghost" onClick={() => stepMatch(-1)} title="Previous">‹</button>
                <span className="match-count">{matchIndex + 1}/{matches.length}</span>
                <button type="button" className="ghost" onClick={() => stepMatch(1)} title="Next">›</button>
              </>
            )}
          </form>
        )}

        <div className="spacer" />
        {file && formats.length > 0 && (
          <span className="format-badge" title={`${formats[0].description} (${formats[0].confidence}%)`}>{formats[0].format}</span>
        )}
        {file && <span className="pill">{endian.toUpperCase()}</span>}
        {file && <span className="pill">16 / row</span>}
        {file && <span className={"valid" + (valid ? " ok" : "")}>● {valid ? "valid" : "no schema"}</span>}

        {file && (
          <div className="edit-tools">
            {dirty && (
              <span className="dirty-badge" title={`${edit?.dirty_count ?? 0} byte(s) changed`}>
                ● {edit?.dirty_count ?? 0} edited
              </span>
            )}
            <button className="ghost" onClick={handleUndo} disabled={!edit?.can_undo} title="Undo (Ctrl+Z)">↶</button>
            <button className="ghost" onClick={handleRedo} disabled={!edit?.can_redo} title="Redo (Ctrl+Y)">↷</button>
            <button className="ghost" onClick={handleRevert} disabled={!dirty} title="Discard all edits">Revert</button>
            <button className="save-btn" onClick={handleSave} disabled={!dirty} title="Save in place (Ctrl+S, backs up .bak)">Save</button>
            <button className="ghost" onClick={handleSaveAs} title="Save a copy">Save As…</button>
          </div>
        )}
      </header>

      {error && <div className="error-bar" onClick={() => setError(null)}>{error}</div>}

      {!file ? (
        <div className="empty-state">
          <h1>Nybble</h1>
          <p>Open a binary file to inspect its structure.</p>
          <button onClick={handleOpen}>Open File…</button>
        </div>
      ) : (
        <>
        <FileMap
          fileLen={file.len}
          root={tree}
          selected={selected}
          activePath={activePath}
          onSelect={selectField}
          onSeek={selectByte}
        />
        <main
          className="cols"
          style={{
            gridTemplateColumns: `${colW.tree}px 6px ${colW.vinspect}px 6px minmax(200px, 1fr) 6px ${colW.right}px`,
          }}
        >
          {/* Column 1 — parse tree */}
          <section className="col col-tree">
            <div className="col-head">Parse tree
              <button className="popout-btn" title="Pop out to its own window" onClick={() => popOut("tree")}>⤢</button>
            </div>
            <div className="col-body">
              {tree ? (
                <StructureTree root={tree} activePath={activePath} colorFor={colorFor} onSelect={selectField} />
              ) : (
                <p className="hint">Parse a schema to see the structure.</p>
              )}
            </div>
          </section>

          <div className="col-splitter" onPointerDown={(e) => startResize("tree", e)} title="Drag to resize" />

          {/* Column 2 — value inspector */}
          <section className="col col-vinspect">
            <div className="col-head">Value inspector
              <button className="popout-btn" title="Pop out to its own window" onClick={() => popOut("vinspect")}>⤢</button>
            </div>
            <div className="col-body">
              <ValueInspector node={selectedNode} raw={rawBytes} onCommit={commitFieldEdit} />
            </div>
          </section>

          <div className="col-splitter" onPointerDown={(e) => startResize("vinspect", e)} title="Drag to resize" />

          {/* Column 3 — hex view */}
          <section className="col col-hex">
            <div className="col-head">
              Hex view
              {selected != null && (
                <span className="sel-label">selection 0x{selected.toString(16).toUpperCase()}</span>
              )}
              <div className="view-toggle">
                <button className={"seg" + (viewMode === "hex" ? " on" : "")} onClick={() => setViewMode("hex")}>Hex</button>
                <button className={"seg" + (viewMode === "text" ? " on" : "")} onClick={() => setViewMode("text")}>Text</button>
              </div>
              <button className="popout-btn" title="Pop out to its own window" onClick={() => popOut("hex")}>⤢</button>
            </div>
            <HexView
              key={file.path}
              fileLen={file.len}
              selected={selected}
              highlight={highlight}
              mode={viewMode}
              colorAt={colorFor}
              isEdited={isEdited}
              editVersion={editVersion}
              onSelect={selectByte}
            />
          </section>

          <div className="col-splitter" onPointerDown={(e) => startResize("right", e)} title="Drag to resize" />

          {/* Column 4 — data preview + schema + extras */}
          <section className="col col-right">
            <div className="rpanel">
              <div className="col-head">Data preview
                <button className="popout-btn" title="Pop out to its own window" onClick={() => popOut("preview")}>⤢</button>
              </div>
              <div className="col-body">
                <DataPreview node={selectedNode} interp={interp} />
                {guesses.length > 0 && (
                  <div className="guesses">
                    {guesses.map((g, i) => (
                      <div key={i} className="guess">
                        <span className="guess-label">{g.label}</span>
                        <span className="guess-detail">{g.detail}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>

            <div className="rpanel schema-panel">
              <div className="col-head">
                Schema
                <div className="schema-controls">
                  {builtin && (
                    <button className="builtin-btn" onClick={handleUseBuiltin} title={`Load the detected ${formats[0]?.format} schema`}>
                      Use {formats[0]?.format}
                    </button>
                  )}
                  <select
                    className="lib-select"
                    value=""
                    onChange={(e) => { handlePickFromLibrary(e.target.value); e.currentTarget.value = ""; }}
                    title="Load a schema from the library"
                  >
                    <option value="">Library…</option>
                    <optgroup label="Built-in">
                      {library.filter((s) => s.source === "builtin").map((s) => (
                        <option key={s.id} value={s.id}>{s.name}</option>
                      ))}
                    </optgroup>
                    {library.some((s) => s.source === "user") && (
                      <optgroup label="My schemas">
                        {library.filter((s) => s.source === "user").map((s) => (
                          <option key={s.id} value={s.id}>{s.name}</option>
                        ))}
                      </optgroup>
                    )}
                  </select>
                  <select value={endian} onChange={(e) => setEndian(e.target.value as Endianness)}>
                    <option value="le">LE</option>
                    <option value="be">BE</option>
                  </select>
                  <button className="ghost" onClick={beginSaveToLibrary} title="Save this schema to your library">Save★</button>
                  <button className="ghost" onClick={handleExportSchema} title="Export to a shareable file">Export</button>
                  <button className="ghost" onClick={() => setShowPack(true)} title="Export as a registry plugin pack (plugin.toml)">Pack…</button>
                  <button className="ghost" onClick={handleImportSchema} title="Import a schema file">Import</button>
                </div>
              </div>
              {savingLib && (
                <div className="lib-save">
                  <input className="lib-save-name" value={libName} onChange={(e) => setLibName(e.target.value)} placeholder="schema name" spellCheck={false} autoFocus />
                  <input className="lib-save-desc" value={libDesc} onChange={(e) => setLibDesc(e.target.value)} placeholder="description (optional)" spellCheck={false} />
                  <button className="vedit-apply" onClick={confirmSaveToLibrary}>Save</button>
                  <button className="ghost" onClick={() => setSavingLib(false)}>Cancel</button>
                </div>
              )}
              {library.some((s) => s.source === "user") && (
                <div className="lib-user-row">
                  <span className="lib-user-label">My schemas:</span>
                  {library.filter((s) => s.source === "user").map((s) => (
                    <span key={s.id} className="lib-chip" title={s.description || s.name}>
                      <button className="lib-chip-load" onClick={() => handlePickFromLibrary(s.id)}>{s.name}</button>
                      <button className="lib-chip-x" title="Remove from library" onClick={() => handleRemoveFromLibrary(s.id)}>×</button>
                    </span>
                  ))}
                </div>
              )}
              <input className="entry-input" value={entry} onChange={(e) => setEntry(e.target.value)} placeholder="entry struct (default: first)" spellCheck={false} />
              <SchemaEditor value={schemaText} onChange={setSchemaText} error={schemaError ?? undefined} />
              {schemaError && <div className="schema-error">{schemaError}</div>}
              {packNotice && (
                <div className="schema-notice" onClick={() => setPackNotice(null)} title="Dismiss">{packNotice}</div>
              )}
              <button className="reparse" onClick={handleParse}>Re-parse</button>
            </div>

            <div className="rpanel extras-panel">
              <div className="col-head">Entropy · strings</div>
              <div className="col-body">
                {entropyData.length > 0 && <EntropyStrip data={entropyData} fileLen={file.len} onSeek={selectByte} />}
                <div className="strings-list">
                  {strings.slice(0, 200).map((s, i) => (
                    <div
                      key={i}
                      className={"string-hit" + (highlight?.start === s.offset ? " active" : "")}
                      onClick={() => { setSelected(s.offset); setHighlight({ start: s.offset, end: s.offset + s.len }); }}
                      title={`0x${s.offset.toString(16)} · ${s.encoding}`}
                    >
                      <span className="string-off">{s.offset.toString(16).padStart(6, "0")}</span>
                      <span className="string-text">{s.text}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </section>
        </main>
        </>
      )}

      <footer className="statusbar">
        {file ? (
          <>
            <span className={"status-dot" + (valid ? " ok" : "")} />
            <span>{valid ? "parsed" : "no schema"}</span>
            {selected != null && <span>· off 0x{selected.toString(16).toUpperCase()}</span>}
            <span>· {file.len.toLocaleString()} B</span>
            <span>· {endian === "le" ? "little-endian" : "big-endian"}</span>
            <div className="spacer" />
            <span>{formats.length > 0 ? formats[0].format : "unknown format"}</span>
          </>
        ) : (
          <span>No file open</span>
        )}
      </footer>

      {showPlugins && (
        <Plugins onClose={() => setShowPlugins(false)} onChanged={handlePluginsChanged} />
      )}
      {showPack && (
        <ExportPack
          schemaText={schemaText}
          entry={entry}
          endian={endian}
          onClose={() => setShowPack(false)}
          onDone={(m) => setPackNotice(m)}
        />
      )}
    </div>
  );
}
