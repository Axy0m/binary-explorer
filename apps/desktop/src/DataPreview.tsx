import type { FieldNode, Interpretations } from "./api";
import { formatValue } from "./StructureTree";

interface Props {
  node: FieldNode | null;
  interp: Interpretations | null;
}

/** The "DATA PREVIEW" panel: a headline value plus byte interpretation chips. */
export function DataPreview({ node, interp }: Props) {
  if (!interp) {
    return <p className="hint">Select a byte or field to preview.</p>;
  }
  const headlineType = node ? node.type_name : "f32 (float)";
  const headline = node ? formatValue(node.value) : fmt(interp.f32_le);

  const beHex =
    interp.u32_be == null ? "—" : "0x" + (interp.u32_be >>> 0).toString(16).toUpperCase().padStart(8, "0");

  return (
    <div className="dpreview">
      <div className="dp-headline">
        <span className="dp-type">{headlineType}</span>
        <span className="dp-value">{headline || "—"}</span>
      </div>
      <div className="dp-chips">
        <Chip label="AS INT32" value={fmt(interp.i32_le)} />
        <Chip label="AS UINT32" value={fmt(interp.u32_le)} />
        <Chip label="AS FLOAT32" value={fmt(interp.f32_le)} />
        <Chip label="BIG-ENDIAN" value={beHex} />
        <Chip label="AS UINT8" value={fmt(interp.u8)} />
        <Chip label="AS CHAR" value={asChar(interp.u8)} />
      </div>
    </div>
  );
}

function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div className="dp-chip">
      <span className="dp-chip-label">{label}</span>
      <span className="dp-chip-value">{value}</span>
    </div>
  );
}

function fmt(v: number | null | undefined): string {
  return v === null || v === undefined ? "—" : String(v);
}

function asChar(v: number | null | undefined): string {
  if (v == null) return "—";
  return v >= 0x20 && v <= 0x7e ? `'${String.fromCharCode(v)}'` : `0x${v.toString(16).padStart(2, "0")}`;
}
