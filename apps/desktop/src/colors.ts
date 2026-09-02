import type { FieldNode } from "./api";

/** Palette used to tint fields — cycles across leaf fields in offset order so
 *  adjacent fields are visually distinct in both the hex view and parse tree. */
const PALETTE = ["var(--accent)", "var(--purple)", "var(--pink)", "var(--amber)", "var(--green)"];

export interface ColorRange {
  start: number;
  end: number;
  color: string;
}

/** Build sorted, non-overlapping color ranges from a parsed tree's leaf fields. */
export function buildColorMap(root: FieldNode | null): ColorRange[] {
  if (!root) return [];
  const leaves: FieldNode[] = [];
  const walk = (n: FieldNode) => {
    if (n.children.length === 0) leaves.push(n);
    else n.children.forEach(walk);
  };
  walk(root);
  leaves.sort((a, b) => a.offset - b.offset);
  return leaves
    .filter((n) => n.size > 0)
    .map((n, i) => ({ start: n.offset, end: n.offset + n.size, color: PALETTE[i % PALETTE.length] }));
}

/** Color for the byte at `offset`, or undefined if it belongs to no field. */
export function colorAt(ranges: ColorRange[], offset: number): string | undefined {
  // Linear scan is fine — a parsed struct rarely has more than a few hundred leaves.
  for (const r of ranges) if (offset >= r.start && offset < r.end) return r.color;
  return undefined;
}
