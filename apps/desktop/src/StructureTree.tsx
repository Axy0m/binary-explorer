import { useState } from "react";
import type { Fault, FieldNode, Value } from "./api";

interface Props {
  root: FieldNode;
  /** Path of the currently active node (see `pathOf`), highlighted in the tree. */
  activePath: string | null;
  /** Color for a node's type dot, keyed by its byte offset (matches the hex view). */
  colorFor?: (offset: number) => string | undefined;
  /** Where the parse stopped, marked in place at the end of the partial tree. */
  fault?: Fault | null;
  onSelect: (node: FieldNode, path: string) => void;
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

export function StructureTree({ root, activePath, colorFor, fault, onSelect }: Props) {
  const faultPath = fault ? faultAnchorPath(root) : null;
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
  onSelect: (node: FieldNode, path: string) => void;
}

function TreeNode({ node, path, depth, activePath, colorFor, fault, faultPath, onSelect }: NodeProps) {
  const [open, setOpen] = useState(depth < 2); // expand the first couple levels
  const hasChildren = node.children.length > 0;
  const isActive = activePath === path;
  // The fault row belongs to the deepest node the parse got into.
  const isFaultSite = faultPath === path;
  // A fault can sit deeper than the levels that open by default, so force every
  // ancestor of it open - a marker you have to go hunting for is no marker.
  const onFaultTrail = faultPath != null && faultPath.startsWith(path + "/");
  const expanded = open || onFaultTrail;

  return (
    <div className="tree-node">
      <div
        className={"tree-row" + (isActive ? " active" : "")}
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
        <span className="tree-value">{formatValue(node.value)}</span>
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
