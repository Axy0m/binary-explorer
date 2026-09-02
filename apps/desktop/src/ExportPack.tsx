// "Export as plugin pack" modal (registry Phase 3 groundwork). Turns the
// schema currently in the editor into a registry-ready `plugin.toml`: metadata
// + one format with a magic-number detect rule and the inline schema. The
// backend validates that the schema parses before writing, so the exported
// pack is guaranteed installable. This is the app -> registry handoff: the file
// it writes drops straight into a `formats/<id>/` folder for a PR.
import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { exportPlugin, readRange, type Endianness } from "./api";

/** A lowercase-slug guess from a struct name, e.g. "PngHeader" -> "png". */
function slugify(s: string): string {
  const base = s.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  return base || "format";
}

export function ExportPack({
  schemaText,
  entry,
  endian,
  onClose,
  onDone,
}: {
  schemaText: string;
  entry: string;
  endian: Endianness;
  onClose: () => void;
  onDone: (msg: string) => void;
}) {
  const guess = entry.trim() || "Format";
  const [id, setId] = useState(slugify(guess));
  const [name, setName] = useState(guess);
  const [formatName, setFormatName] = useState(guess.toUpperCase());
  const [extension, setExtension] = useState("");
  const [author, setAuthor] = useState("");
  const [description, setDescription] = useState("");
  const [confidence, setConfidence] = useState(90);
  const [offset, setOffset] = useState(0);
  const [hex, setHex] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Prefill the magic from the open file's first bytes — the common case is a
  // signature at offset 0. The user trims it to the bytes that are actually
  // constant for the format.
  useEffect(() => {
    (async () => {
      try {
        const bytes = await readRange(0, 8);
        if (bytes.length) {
          setHex(Array.from(bytes).map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join(" "));
        }
      } catch {
        /* no file open — leave the magic blank */
      }
    })();
  }, []);

  async function doExport() {
    setErr(null);
    try {
      const path = await save({
        title: "Export plugin pack",
        defaultPath: "plugin.toml",
        filters: [{ name: "Plugin", extensions: ["toml"] }],
      });
      if (typeof path !== "string") return;
      setBusy(true);
      await exportPlugin({
        path,
        id: id.trim(),
        name: name.trim(),
        version: "1.0.0",
        author,
        description,
        formatName,
        extension,
        entry,
        endian,
        confidence,
        detectOffset: offset,
        detectHex: hex,
        schemaText,
      });
      onDone(`Exported ${id.trim()} plugin pack.`);
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal pack-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Export as plugin pack</h2>
          <button className="modal-x" onClick={onClose} title="Close">×</button>
        </div>

        <p className="plugins-intro">
          Writes a registry-ready <code>plugin.toml</code> from the current
          schema. Drop it into a <code>formats/&lt;id&gt;/</code> folder in the
          registry and open a PR. The schema is validated before export.
        </p>

        <div className="pack-grid">
          <label>Id (folder name)</label>
          <input value={id} onChange={(e) => setId(e.target.value)} spellCheck={false} placeholder="png" />

          <label>Display name</label>
          <input value={name} onChange={(e) => setName(e.target.value)} spellCheck={false} placeholder="PNG image" />

          <label>Format name</label>
          <input value={formatName} onChange={(e) => setFormatName(e.target.value)} spellCheck={false} placeholder="PNG" />

          <label>Extension</label>
          <input value={extension} onChange={(e) => setExtension(e.target.value)} spellCheck={false} placeholder="png" />

          <label>Description</label>
          <input value={description} onChange={(e) => setDescription(e.target.value)} spellCheck={false} placeholder="one line" />

          <label>Author</label>
          <input value={author} onChange={(e) => setAuthor(e.target.value)} spellCheck={false} placeholder="your name / handle" />

          <label>Magic (hex)</label>
          <input value={hex} onChange={(e) => setHex(e.target.value)} spellCheck={false} placeholder="89 50 4E 47  (blank = no auto-detect)" />

          <label>Magic offset</label>
          <input type="number" value={offset} min={0} onChange={(e) => setOffset(Number(e.target.value) || 0)} />

          <label>Confidence</label>
          <input type="number" value={confidence} min={0} max={100} onChange={(e) => setConfidence(Math.max(0, Math.min(100, Number(e.target.value) || 0)))} />

          <label>Endianness</label>
          <div className="pack-static">{endian === "be" ? "big-endian (BE)" : "little-endian (LE)"} · entry: {entry.trim() || "(first struct)"}</div>
        </div>

        {err && <div className="plugins-err">{err}</div>}

        <div className="pack-actions">
          <button className="pg-install" onClick={doExport} disabled={busy}>
            {busy ? "Exporting…" : "Export plugin.toml…"}
          </button>
          <button className="ghost" onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
