import { useMemo, useState } from "react";
import type { Fault, FieldNode, Value } from "./api";

interface Props {
  root: FieldNode;
  /** Path of the currently active node (see `pathOf`), highlighted in the tree. */
  activePath: string | null;
  /** Color for a node's type dot, keyed by its byte offset (matches the hex view). */
  colorFor?: (offset: number) => string | undefined;
  /** Where the parse stopped, marked in place at the end of the partial tree. */
  fault?: Fault | null;
  /** The same schema run against the file being compared against, if any. Nodes
   *  whose value differs then show `other -> mine`. */
  otherRoot?: FieldNode | null;
  onSelect: (node: FieldNode, path: string) => void;
}

/** What a comparison says about each node: its counterpart, and whether it (or
 *  anything under it) differs. */
interface Comparison {
  others: Map<string, FieldNode>;
  changed: Set<string>;
}

/**
 * Pair up two runs of the same schema, node by node.
 *
 * The two trees are walked in lockstep by position, which is what makes the
 * pairing meaningful: same schema, same field order. Where the shapes diverge —
 * a length field that changed made one side parse a different number of
 * elements — the unpaired nodes simply get no counterpart, rather than being
 * lined up with whatever happens to sit at the same index.
 */
function compareTrees(mine: FieldNode, other: FieldNode | null): Comparison {
  const out: Comparison = { others: new Map(), changed: new Set() };
  walkPair(mine, other, ROOT_PATH, out);
  return out;
}

function walkPair(mine: FieldNode, other: FieldNode | null, path: string, out: Comparison): boolean {
  if (other == null) return false;
  out.others.set(path, other);

  // A leaf differs when its decoded value does; a container differs when
  // anything under it does. Span changes count too: a field that moved or
  // resized is a change even if the value it decoded to happens to match.
  let changed = mine.size !== other.size || mine.offset !== other.offset;
  if (mine.children.length === 0 && other.children.length === 0) {
    if (formatValue(mine.value) !== formatValue(other.value)) changed = true;
  }
  if (mine.children.length !== other.children.length) changed = true;
  for (let i = 0; i < mine.children.length; i++) {
    if (walkPair(mine.children[i], other.children[i] ?? null, childPath(path, i), out)) {
      changed = true;
    }
  }
  if (changed) out.changed.add(path);
  return changed;
}

/**
 * Path of the node the parse stopped inside.
 *
 * The first fault breaks out of every enclosing container, so the site is
 * always reachable by descending the last child from the root - no extra
 * bookkeeping needed to place the marker.
 */
export function faultAnchorPath(root: FieldNode): string {
  let node = root;
  let path = ROOT_PATH;
  while (node.children.length > 0) {
    const i = node.children.length - 1;
    node = node.children[i];
    path = childPath(path, i);
  }
  return path;
}

/** Stable identity for a node: its position in the tree, e.g. "r/2/0". */
function childPath(parentPath: string, index: number): string {
  return `${parentPath}/${index}`;
}

export const ROOT_PATH = "r";

/**
 * Find the deepest field whose byte range contains `offset`, returning it and
 * its path — this is how a click in the hex view selects the matching field.
 */
export function findFieldAtOffset(
  root: FieldNode,
  offset: number,
): { node: FieldNode; path: string } | null {
  // Fast path: descend the contiguous nesting from the root.
  if (offset >= root.offset && offset < root.offset + root.size) {
    let node = root;
    let path = ROOT_PATH;
    outer: while (node.children.length > 0) {
      for (let i = 0; i < node.children.length; i++) {
        const c = node.children[i];
        if (offset >= c.offset && offset < c.offset + c.size) {
          node = c;
          path = childPath(path, i);
          continue outer;
        }
      }
      break; // offset falls in this node but in none of its children (padding)
    }
    return { node, path };
  }

  // Pointer targets live outside the root's contiguous span, so the descent
  // above can't reach them. Fall back to a full-tree search for the smallest
  // field that contains the offset.
  let best: { node: FieldNode; path: string } | null = null;
  const walk = (n: FieldNode, path: string) => {
    if (n.size > 0 && offset >= n.offset && offset < n.offset + n.size) {
      if (!best || n.size < best.node.size) best = { node: n, path };
    }
    n.children.forEach((c, i) => walk(c, childPath(path, i)));
  };
  walk(root, ROOT_PATH);
  return best;
}

/** How many `check` fields in the tree disagree with the bytes they cover. */
export function countBadChecks(node: FieldNode): number {
  let n = node.check && !node.check.ok ? 1 : 0;
  for (const child of node.children) n += countBadChecks(child);
  return n;
}

export function StructureTree({ root, activePath, colorFor, fault, otherRoot, onSelect }: Props) {
  const faultPath = fault ? faultAnchorPath(root) : null;
  const cmp = useMemo(
    () => (otherRoot ? compareTrees(root, otherRoot) : null),
    [root, otherRoot],
  );
  return (
    <div className="tree">
      <TreeNode
        node={root}
        path={ROOT_PATH}
        depth={0}
        activePath={activePath}
        colorFor={colorFor}
        fault={fault ?? null}
        faultPath={faultPath}
        cmp={cmp}
        onSelect={onSelect}
      />
    </div>
  );
}

interface NodeProps {
  node: FieldNode;
  path: string;
  depth: number;
  activePath: string | null;
  colorFor?: (offset: number) => string | undefined;
  fault: Fault | null;
  faultPath: string | null;
  cmp: Comparison | null;
  onSelect: (node: FieldNode, path: string) => void;
}

function TreeNode({ node, path, depth, activePath, colorFor, fault, faultPath, cmp, onSelect }: NodeProps) {
  const [open, setOpen] = useState(depth < 2); // expand the first couple levels
  const hasChildren = node.children.length > 0;
  const isActive = activePath === path;
  // The fault row belongs to the deepest node the parse got into.
  const isFaultSite = faultPath === path;
  // A fault can sit deeper than the levels that open by default, so force every
  // ancestor of it open - a marker you have to go hunting for is no marker.
  const onFaultTrail = faultPath != null && faultPath.startsWith(path + "/");
  // A change in a collapsed subtree is a change you would never find, so the
  // trail down to one opens itself - same reasoning as the fault trail.
  const onChangeTrail =
    cmp != null && cmp.changed.has(path) && node.children.length > 0 && depth < 6;
  const expanded = open || onFaultTrail || onChangeTrail;

  const other = cmp?.others.get(path);
  const isChanged = cmp?.changed.has(path) ?? false;
  // Only a leaf shows a before value; for a container the changed children do.
  const wasValue =
    isChanged && other != null && node.children.length === 0 && other.children.length === 0
      ? formatValue(other.value)
      : null;

  return (
    <div className="tree-node">
      <div
        className={"tree-row" + (isActive ? " active" : "") + (isChanged ? " changed" : "")}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() => onSelect(node, path)}
      >
        <span
          className={"twisty" + (hasChildren ? "" : " leaf")}
          onClick={(e) => {
            e.stopPropagation();
            if (hasChildren) setOpen((o) => !o);
          }}
        >
          {hasChildren ? (expanded ? "▾" : "▸") : "·"}
        </span>
        <span className="tree-dot" style={{ background: colorFor?.(node.offset) ?? "var(--muted-2)" }} />
        <span className="tree-name">{node.name}</span>
        <span className="tree-type">{node.type_name}</span>
        {node.description && (
          <span className="tree-desc" title={node.description}>
            {node.description}
          </span>
        )}
        {wasValue != null && (
          <span className="tree-was" title="value in the compared file">
            {wasValue} <span className="tree-arrow">→</span>
          </span>
        )}
        <span className="tree-value">{formatValue(node.value)}</span>
        {node.check && (
          <span
            className={"tree-check" + (node.check.ok ? " ok" : " bad")}
            title={
              node.check.ok
                ? `${node.check.algo} matches the ${node.check.over_size} bytes at 0x${node.check.over_offset.toString(16).toUpperCase()}`
                : `${node.check.algo} mismatch — those ${node.check.over_size} bytes produce 0x${node.check.computed.toString(16).toUpperCase()}`
            }
          >
            {node.check.ok
              ? "✓"
              : `✗ 0x${node.check.computed.toString(16).toUpperCase()}`}
          </span>
        )}
      </div>
      {hasChildren && expanded && (
        <div className="tree-children">
          {node.children.map((c, i) => (
            <TreeNode
              key={i}
              node={c}
              path={childPath(path, i)}
              depth={depth + 1}
              activePath={activePath}
              colorFor={colorFor}
              fault={fault}
              faultPath={faultPath}
              cmp={cmp}
              onSelect={onSelect}
            />
          ))}
        </div>
      )}
      {isFaultSite && fault && (
        <div className="tree-row fault" style={{ paddingLeft: 8 + (depth + 1) * 14 }} title={fault.message}>
          <span className="twisty leaf">✗</span>
          <span className="tree-name">{lastSegment(fault.path)}</span>
          <span className="tree-value">{fault.message}</span>
        </div>
      )}
    </div>
  );
}

/** The trailing `name` (or `[i]`) of a fault path, for the in-tree marker. */
function lastSegment(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot === -1 ? path : path.slice(dot + 1);
}

/** Human-readable rendering of a decoded value for the structure tree. */
export function formatValue(v: Value): string {
  switch (v.kind) {
    case "u":
    case "i":
      return String(v.value);
    case "f":
      return String(v.value);
    case "bool":
      return v.value ? "true" : "false";
    case "char":
      return `'${v.value}'`;
    case "str":
      return JSON.stringify(v.value); // quoted, escapes control chars
    case "bytes":
      return v.value.map((b) => b.toString(16).padStart(2, "0")).join(" ");
    case "enum":
      return v.value.name != null ? `${v.value.name} (${v.value.value})` : `${v.value.value} (unknown)`;
    case "struct":
    case "array":
    case "bitfield":
      return "";
  }
}
