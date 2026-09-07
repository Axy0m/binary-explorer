// Programmatic edits to the schema DSL text.
//
// The hex view's "make this a field" flow drops new field lines into an
// existing struct. It works on the source text rather than on a parsed AST so
// that comments, spacing and anything else the user typed survive untouched.

/** A field line to append: `name  type`. */
export interface NewField {
  name: string;
  type: string;
}

const STRUCT_RE = /\bstruct\s+([A-Za-z_]\w*)\s*\{/g;
/** A field line already in a struct body: indent, name, gap, then its type. */
const FIELD_RE = /^([ \t]*)([A-Za-z_]\w*)([ \t]+)\S/gm;

/** Column the type starts in when a struct has no fields to copy the layout from. */
const DEFAULT_TYPE_COL = 10;

interface StructHit {
  name: string;
  /** Index of the `{` that opens the body. */
  open: number;
}

/** The struct a new field joins: `entry` if the schema defines it, else the first. */
function findStruct(text: string, entry: string): StructHit | null {
  const want = entry.trim();
  let first: StructHit | null = null;
  STRUCT_RE.lastIndex = 0;
  for (let m = STRUCT_RE.exec(text); m !== null; m = STRUCT_RE.exec(text)) {
    const hit: StructHit = { name: m[1], open: m.index + m[0].length - 1 };
    if (first == null) first = hit;
    if (want !== "" && m[1] === want) return hit;
  }
  return first;
}

/**
 * Index of the `}` closing the block opened at `open`, or -1 if unbalanced.
 * Braces inside `"strings"` and `// comments` don't count towards the depth.
 */
function matchBrace(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      const close = text.indexOf('"', i + 1);
      if (close < 0) return -1;
      i = close;
    } else if (c === "/" && text[i + 1] === "/") {
      const nl = text.indexOf("\n", i);
      if (nl < 0) return -1;
      i = nl;
    } else if (c === "{") {
      depth++;
    } else if (c === "}") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** `name`, padded so the type lands in column `col` (always at least one space). */
function pad(name: string, col: number): string {
  return name + " ".repeat(Math.max(1, col - name.length));
}

/** `base`, or `base_2`, `base_3`… if the struct already has a field by that name. */
function uniqueName(base: string, taken: Set<string>): string {
  if (!taken.has(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}_${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** Name of the struct `insertFields` would append to (for labelling the UI). */
export function targetStructName(text: string, entry: string): string {
  return findStruct(text, entry)?.name ?? (entry.trim() || "File");
}

/**
 * Append `fields` to the end of the target struct's body, matching the indent
 * and name-column width already in use there. If the schema has no struct at
 * all, one is created. Returns the text unchanged when the source has an
 * unbalanced brace — a broken schema is the user's to fix, not ours to rewrite.
 */
export function insertFields(text: string, entry: string, fields: NewField[]): string {
  if (fields.length === 0) return text;

  const target = findStruct(text, entry);
  if (target == null) {
    const lines = fields.map((f) => `    ${pad(f.name, DEFAULT_TYPE_COL)}${f.type}`);
    const head = text.trim() === "" ? "" : `${text.replace(/\s*$/, "")}\n\n`;
    return `${head}struct ${entry.trim() || "File"} {\n${lines.join("\n")}\n}\n`;
  }

  const close = matchBrace(text, target.open);
  if (close < 0) return text;

  const body = text.slice(target.open + 1, close);
  FIELD_RE.lastIndex = 0;
  const existing = [...body.matchAll(FIELD_RE)];
  const indent = existing.length > 0 ? existing[0][1] : "    ";
  // Line the types up with the ones already there.
  const col = existing.length > 0
    ? Math.max(...existing.map((m) => m[2].length + m[3].length))
    : DEFAULT_TYPE_COL;
  const taken = new Set(existing.map((m) => m[2]));

  const lines = fields.map((f) => {
    const name = uniqueName(f.name, taken);
    taken.add(name);
    return `${indent}${pad(name, col)}${f.type}`;
  });

  // Keep the closing brace on its own line with whatever indent it had.
  const before = body.replace(/[ \t]*$/, "");
  const sep = before.endsWith("\n") ? "" : "\n";
  return (
    text.slice(0, target.open + 1) +
    before + sep + lines.join("\n") + "\n" +
    text.slice(close)
  );
}
