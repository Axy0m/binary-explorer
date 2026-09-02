import type { FieldNode } from "./api";
import { ROOT_PATH } from "./StructureTree";

interface Props {
  fileLen: number;
  /** Parsed tree; its top-level fields become the map's segments. */
  root: FieldNode | null;
  /** Currently selected byte offset — drawn as a marker line, and used to
   *  highlight whichever segment contains it (even for a nested field). */
  selected: number | null;
  /** Path of the active field, so the matching segment can be highlighted. */
  activePath: string | null;
  /** Select a field (click a segment). */
  onSelect: (node: FieldNode, path: string) => void;
  /** Jump to a byte offset (click an empty part of the track). */
  onSeek: (offset: number) => void;
}

/** Same family as the hex view / tree palette, cycled per top-level segment. */
const PALETTE = ["var(--accent)", "var(--purple)", "var(--pink)", "var(--amber)", "var(--green)"];

/** Compact byte size, e.g. 128, 5.0 KB, 20 MB. */
function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function fmtOffset(n: number): string {
  return "0x" + n.toString(16).toUpperCase();
}

/**
 * File-map ribbon — a proportional, full-width band showing where each
 * top-level field lives in the file. Segments are colored and clickable;
 * gaps are unparsed regions; a marker tracks the current selection. This is
 * the "visualize the file once you understand its structure" view.
 */
export function FileMap({ fileLen, root, selected, activePath, onSelect, onSeek }: Props) {
  const segments = (root?.children ?? [])
    .map((node, i) => ({ node, path: `${ROOT_PATH}/${i}` }))
    .filter(({ node }) => node.size > 0);

  // Click on the bare track jumps to the proportional offset.
  function onTrackClick(e: React.MouseEvent<HTMLDivElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    onSeek(Math.min(fileLen - 1, Math.floor(frac * fileLen)));
  }

  const pct = (n: number) => `${(n / fileLen) * 100}%`;

  return (
    <div className="filemap">
      <div className="filemap-track" onClick={onTrackClick}>
        {segments.map(({ node, path }, i) => {
          // Highlight the segment when it (or a field nested inside it) is the
          // current selection.
          const active =
            activePath === path ||
            (selected != null && selected >= node.offset && selected < node.offset + node.size);
          return (
            <button
              key={path}
              className={"filemap-seg" + (active ? " active" : "")}
              style={{ left: pct(node.offset), width: pct(node.size), background: PALETTE[i % PALETTE.length] }}
              title={`${node.name}  ${node.type_name}\n${fmtOffset(node.offset)} – ${fmtOffset(node.offset + node.size)}  (${fmtSize(node.size)})`}
              onClick={(e) => { e.stopPropagation(); onSelect(node, path); }}
            >
              <span className="filemap-seg-label">{node.name}</span>
            </button>
          );
        })}
        {selected != null && fileLen > 0 && (
          <div className="filemap-marker" style={{ left: pct(selected) }} title={`selection ${fmtOffset(selected)}`} />
        )}
      </div>
      <div className="filemap-axis">
        <span>0</span>
        {segments.length === 0 && <span className="filemap-hint">parse a schema to map the file</span>}
        <span>{fmtSize(fileLen)}</span>
      </div>
    </div>
  );
}
