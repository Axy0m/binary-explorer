import { useEffect, useState } from "react";
import { isEditableKind, type FieldNode, type Value } from "./api";
import { formatValue } from "./StructureTree";

interface Props {
  node: FieldNode | null;
  /** Raw bytes of the selected field (may be truncated for large fields). */
  raw: Uint8Array | null;
  /** Commit an edited value for the current node; resolves to an error or null. */
  onCommit?: (value: string) => Promise<string | null>;
}

/** The "VALUE INSPECTOR" column: the selected field's details, one row each. */
export function ValueInspector({ node, raw, onCommit }: Props) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Leaving a field (or its value changing) drops any in-progress edit.
  useEffect(() => {
    setEditing(false);
    setErr(null);
  }, [node?.offset, node?.size, node?.type_name]);

  if (!node) {
    return <p className="hint">Select a field in the parse tree.</p>;
  }
  const hex = raw ? [...raw].map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join(" ") : "—";
  const bin = raw ? [...raw].map((b) => b.toString(2).padStart(8, "0")).join(" ") : "—";
  const editable = onCommit != null && isEditableKind(node.value.kind);

  function startEdit() {
    setDraft(editableText(node!.value));
    setErr(null);
    setEditing(true);
  }

  async function commit() {
    if (!onCommit) return;
    setBusy(true);
    const message = await onCommit(draft);
    setBusy(false);
    if (message) {
      setErr(message);
    } else {
      setEditing(false);
      setErr(null);
    }
  }

  return (
    <div className="vinspect">
      <div className="vinspect-title">
        <span className="dot" style={{ background: "var(--accent)" }} />
        {node.name}
      </div>
      <Row label="Offset" value={`0x${node.offset.toString(16).toUpperCase().padStart(8, "0")} (${node.offset})`} />
      <Row label="Size" value={`${node.size} bytes`} />
      <Row label="Type" value={node.type_name} />

      {editing ? (
        <div className="vedit">
          <input
            className="vedit-input"
            value={draft}
            autoFocus
            spellCheck={false}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") setEditing(false);
            }}
          />
          <div className="vedit-actions">
            <button className="vedit-apply" disabled={busy} onClick={commit}>Apply</button>
            <button className="ghost" disabled={busy} onClick={() => setEditing(false)}>Cancel</button>
          </div>
          {err && <div className="vedit-error">{err}</div>}
        </div>
      ) : (
        <div className="vrow">
          <span className="vrow-label">Value</span>
          <span className="vrow-val strong">{formatValue(node.value) || "—"}</span>
          {editable && (
            <button className="vedit-pencil" title="Edit value" onClick={startEdit}>Edit</button>
          )}
        </div>
      )}

      <Row label="Raw bytes" value={hex} mono />
      <Row label="Binary" value={bin} mono />
      <Row label="Description" value={node.description || "—"} />
    </div>
  );
}

/** The editable string form of a value (what pre-fills the edit input). */
function editableText(value: Value): string {
  switch (value.kind) {
    case "u":
    case "i":
    case "f":
      return String(value.value);
    case "bool":
      return value.value ? "true" : "false";
    case "char":
    case "str":
      return value.value;
    case "bytes":
      return value.value.map((b) => b.toString(16).padStart(2, "0")).join(" ");
    default:
      return "";
  }
}

function Row({ label, value, mono, strong }: { label: string; value: string; mono?: boolean; strong?: boolean }) {
  return (
    <div className="vrow">
      <span className="vrow-label">{label}</span>
      <span className={"vrow-val" + (mono ? " mono" : "") + (strong ? " strong" : "")}>{value}</span>
    </div>
  );
}
