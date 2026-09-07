import { useEffect, useMemo, useRef, useState } from "react";
import type { Endianness } from "./api";

export interface Range {
  start: number;
  end: number;
}

interface Props {
  /** The byte range picked in the hex view, `[start, end)`. */
  range: Range;
  /** Bytes of the range (capped for long selections — see `PREVIEW_CAP`). */
  bytes: Uint8Array | null;
  endian: Endianness;
  /** Struct the field will be appended to. */
  target: string;
  /** Unclaimed bytes between the end of the current parse and the selection. */
  gap: number;
  /** True when the selection sits inside bytes the schema already covers. */
  overlaps: boolean;
  onAdd: (name: string, type: string, pad: number) => void;
  onCancel: () => void;
}

/** How many bytes of a selection are fetched to preview candidate readings. */
export const PREVIEW_CAP = 64;

interface Candidate {
  type: string;
  preview: string;
}

/**
 * The bar under the hex view: turns a dragged byte range into a schema field.
 * It offers the readings the bytes actually support — printable runs get
 * `char[n]` first, four bytes get `u32`/`i32`/`f32`, and `bytes[n]` always
 * works — so a schema can be built by selecting and clicking, without knowing
 * the DSL up front.
 */
export function FieldBuilder({ range, bytes, endian, target, gap, overlaps, onAdd, onCancel }: Props) {
  const len = range.end - range.start;
  const cands = useMemo(() => candidates(bytes, len, endian), [bytes, len, endian]);
  const suggested = useMemo(() => suggestName(bytes, len, range.start), [bytes, len, range.start]);

  const [type, setType] = useState(cands[0]?.type ?? `bytes[${len}]`);
  const [name, setName] = useState(suggested);
  const [pad, setPad] = useState(true);
  const nameRef = useRef<HTMLInputElement>(null);
  const touched = useRef(false);

  // The selection's bytes arrive a tick after the range does. Adopt the better
  // defaults they reveal (a name from the text, a numeric type) unless the
  // user already picked something. A new selection remounts this component,
  // so the draft never leaks from one range to the next.
  useEffect(() => {
    if (touched.current || bytes == null) return;
    setType(cands[0]?.type ?? `bytes[${len}]`);
    setName(suggested);
  }, [bytes, cands, suggested, len]);

  useEffect(() => nameRef.current?.select(), []);

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const clean = name.trim().replace(/[^A-Za-z0-9_]/g, "_") || `field_${range.start.toString(16)}`;
    onAdd(clean, type, pad ? gap : 0);
  }

  return (
    <div className="fieldbuilder">
      <div className="fb-head">
        <span className="fb-range">
          0x{range.start.toString(16).toUpperCase()}–0x{range.end.toString(16).toUpperCase()}
        </span>
        <span className="fb-len">{len} {len === 1 ? "byte" : "bytes"}</span>
        <span className="fb-target">→ struct {target}</span>
        <button className="fb-x" onClick={onCancel} title="Discard the selection (Esc)">×</button>
      </div>

      <div className="fb-cands">
        {cands.map((c) => (
          <button
            key={c.type}
            className={"fb-cand" + (c.type === type ? " on" : "")}
            onClick={() => { touched.current = true; setType(c.type); }}
            title={`Read these bytes as ${c.type}`}
          >
            <span className="fb-cand-type">{c.type}</span>
            <span className="fb-cand-prev">{c.preview}</span>
          </button>
        ))}
      </div>

      <form className="fb-add" onSubmit={submit}>
        <input
          ref={nameRef}
          className="fb-name"
          value={name}
          onChange={(e) => { touched.current = true; setName(e.target.value); }}
          onKeyDown={(e) => e.key === "Escape" && onCancel()}
          placeholder="field name"
          spellCheck={false}
          autoFocus
        />
        {gap > 0 && (
          <label className="fb-pad" title={`${gap} byte(s) between the end of the parse and this selection`}>
            <input type="checkbox" checked={pad} onChange={(e) => setPad(e.target.checked)} />
            pad {gap} B
          </label>
        )}
        <button type="submit" className="fb-go">Add field</button>
      </form>

      {overlaps && (
        <div className="fb-warn">
          These bytes are already covered by the parse — the field is still appended to the end of {target}.
        </div>
      )}
    </div>
  );
}

/** Readings the selected bytes support, most plausible first. */
function candidates(bytes: Uint8Array | null, len: number, endian: Endianness): Candidate[] {
  const out: Candidate[] = [];
  if (len <= 0) return out;

  const hexOf = (b: Uint8Array) =>
    [...b].map((x) => x.toString(16).padStart(2, "0").toUpperCase()).join(" ");

  if (bytes != null && bytes.length > 0) {
    const whole = bytes.length === len; // false once a long selection was truncated
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const le = endian === "le";
    const ascii = (b: Uint8Array) => [...b].every((x) => x >= 0x20 && x <= 0x7e);

    if (whole && ascii(bytes)) {
      out.push({ type: `char[${len}]`, preview: `"${text(bytes)}"` });
    }
    if (whole && len > 1 && bytes[len - 1] === 0 && ascii(bytes.subarray(0, len - 1))) {
      out.push({ type: "cstring", preview: `"${text(bytes.subarray(0, len - 1))}"` });
    }

    if (len === 1) {
      out.push({ type: "u8", preview: String(view.getUint8(0)) });
      out.push({ type: "i8", preview: String(view.getInt8(0)) });
      out.push({ type: "bool", preview: bytes[0] === 0 ? "false" : "true" });
    } else if (len === 2) {
      out.push({ type: "u16", preview: String(view.getUint16(0, le)) });
      out.push({ type: "i16", preview: String(view.getInt16(0, le)) });
    } else if (len === 4) {
      out.push({ type: "u32", preview: String(view.getUint32(0, le)) });
      out.push({ type: "i32", preview: String(view.getInt32(0, le)) });
      out.push({ type: "f32", preview: float(view.getFloat32(0, le)) });
    } else if (len === 8) {
      out.push({ type: "u64", preview: String(view.getBigUint64(0, le)) });
      out.push({ type: "i64", preview: String(view.getBigInt64(0, le)) });
      out.push({ type: "f64", preview: float(view.getFloat64(0, le)) });
    }
  }

  if (len > 8 && len % 4 === 0) {
    out.push({ type: `u32[${len / 4}]`, preview: `${len / 4} × u32` });
  }
  out.push({
    type: `bytes[${len}]`,
    preview: bytes == null ? "…" : hexOf(bytes.subarray(0, 6)) + (len > 6 ? " …" : ""),
  });
  return out;
}

/**
 * A starting name for the field. A printable run that reads as an identifier
 * names itself (selecting `IHDR` gives `ihdr`); anything else falls back to
 * its offset.
 */
function suggestName(bytes: Uint8Array | null, len: number, start: number): string {
  const fallback = `field_${start.toString(16)}`;
  if (bytes == null || bytes.length !== len || len === 0 || len > 24) return fallback;
  const s = text(bytes).trim();
  return /^[A-Za-z_]\w*$/.test(s) ? s.toLowerCase() : fallback;
}

function text(b: Uint8Array): string {
  return String.fromCharCode(...b);
}

/** Floats read out of arbitrary bytes are usually junk — keep them short. */
function float(v: number): string {
  if (!Number.isFinite(v)) return String(v);
  if (v !== 0 && (Math.abs(v) < 1e-4 || Math.abs(v) >= 1e9)) return v.toExponential(3);
  return String(Number(v.toFixed(4)));
}
