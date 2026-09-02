import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { readRange } from "./api";

const BYTES_PER_ROW = 16;
const ROW_HEIGHT = 20; // px, must match .hex-row height in styles.css
const PAGE_BYTES = 4096; // fetch the file in 4 KB pages
const OVERSCAN_ROWS = 8;

interface Props {
  fileLen: number;
  selected: number | null;
  /** A byte range [start, end) to highlight (the selected field's span). */
  highlight: { start: number; end: number } | null;
  /** "hex" shows the hex dump; "text" shows only the decoded characters. */
  mode: "hex" | "text";
  /** Tint for the byte at an offset (the field it belongs to); undefined = none. */
  colorAt?: (offset: number) => string | undefined;
  /** True if the byte at an offset has an unsaved edit (marks it in the view). */
  isEdited?: (offset: number) => boolean;
  /** Bumped after any edit/undo/redo so cached byte pages are re-fetched. */
  editVersion?: number;
  onSelect: (offset: number) => void;
}

/**
 * A virtualized hex viewer. Only the visible rows are rendered, and file bytes
 * are fetched from the Rust backend one 4 KB page at a time and cached — so
 * scrolling through a multi-GB file never loads it into memory.
 */
export function HexView({ fileLen, selected, highlight, mode, colorAt, isEdited, editVersion = 0, onSelect }: Props) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(400);

  // Page cache. A version counter forces a re-render when a page arrives.
  const pagesRef = useRef<Map<number, Uint8Array>>(new Map());
  const inflightRef = useRef<Set<number>>(new Set());
  const lastEditVersion = useRef(editVersion);
  const [, setPageVersion] = useState(0);

  const totalRows = Math.max(1, Math.ceil(fileLen / BYTES_PER_ROW));

  // Track the viewport height so virtualization math is correct.
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const measure = () => setViewportHeight(el.clientHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const firstVisibleRow = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN_ROWS);
  const visibleRowCount = Math.ceil(viewportHeight / ROW_HEIGHT) + OVERSCAN_ROWS * 2;
  const lastVisibleRow = Math.min(totalRows, firstVisibleRow + visibleRowCount);

  // Fetch any pages needed to render the visible rows. When an edit lands
  // (editVersion changes) the cached pages are stale, so drop them first.
  useEffect(() => {
    if (lastEditVersion.current !== editVersion) {
      pagesRef.current.clear();
      inflightRef.current.clear();
      lastEditVersion.current = editVersion;
    }
    const firstByte = firstVisibleRow * BYTES_PER_ROW;
    const lastByte = lastVisibleRow * BYTES_PER_ROW;
    const firstPage = Math.floor(firstByte / PAGE_BYTES);
    const lastPage = Math.floor(Math.max(firstByte, lastByte - 1) / PAGE_BYTES);

    for (let page = firstPage; page <= lastPage; page++) {
      if (pagesRef.current.has(page) || inflightRef.current.has(page)) continue;
      inflightRef.current.add(page);
      const offset = page * PAGE_BYTES;
      const length = Math.min(PAGE_BYTES, fileLen - offset);
      if (length <= 0) {
        inflightRef.current.delete(page);
        continue;
      }
      readRange(offset, length)
        .then((bytes) => {
          pagesRef.current.set(page, bytes);
          setPageVersion((v) => v + 1);
        })
        .catch((e) => console.error("read_range failed", e))
        .finally(() => inflightRef.current.delete(page));
    }
  }, [firstVisibleRow, lastVisibleRow, fileLen, editVersion]);

  // Keep the selected byte on screen when it changes (used by "jump to offset").
  useEffect(() => {
    if (selected == null) return;
    const el = scrollerRef.current;
    if (!el) return;
    const row = Math.floor(selected / BYTES_PER_ROW);
    const rowTop = row * ROW_HEIGHT;
    const rowBottom = rowTop + ROW_HEIGHT;
    if (rowTop < el.scrollTop || rowBottom > el.scrollTop + el.clientHeight) {
      el.scrollTop = Math.max(0, rowTop - el.clientHeight / 2);
    }
  }, [selected]);

  const byteAt = useCallback((offset: number): number | undefined => {
    const page = Math.floor(offset / PAGE_BYTES);
    const arr = pagesRef.current.get(page);
    if (!arr) return undefined;
    const idx = offset - page * PAGE_BYTES;
    return idx < arr.length ? arr[idx] : undefined;
  }, []);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop);
  };

  const rows = [];
  for (let row = firstVisibleRow; row < lastVisibleRow; row++) {
    rows.push(
      <HexRow
        key={row}
        row={row}
        fileLen={fileLen}
        byteAt={byteAt}
        selected={selected}
        highlight={highlight}
        mode={mode}
        colorAt={colorAt}
        isEdited={isEdited}
        onSelect={onSelect}
      />,
    );
  }

  return (
    <div className="hex-scroller" ref={scrollerRef} onScroll={onScroll}>
      <div className="hex-canvas" style={{ height: totalRows * ROW_HEIGHT }}>
        {rows}
      </div>
    </div>
  );
}

interface RowProps {
  row: number;
  fileLen: number;
  byteAt: (offset: number) => number | undefined;
  selected: number | null;
  highlight: { start: number; end: number } | null;
  mode: "hex" | "text";
  colorAt?: (offset: number) => string | undefined;
  isEdited?: (offset: number) => boolean;
  onSelect: (offset: number) => void;
}

function HexRow({ row, fileLen, byteAt, selected, highlight, mode, colorAt, isEdited, onSelect }: RowProps) {
  const base = row * BYTES_PER_ROW;
  const hexCells = [];
  const asciiCells = [];

  for (let i = 0; i < BYTES_PER_ROW; i++) {
    const offset = base + i;
    const inFile = offset < fileLen;
    const b = inFile ? byteAt(offset) : undefined;
    const isSel = selected === offset;
    const inRange = highlight != null && offset >= highlight.start && offset < highlight.end;
    const edited = inFile && isEdited != null && isEdited(offset);
    const cls = (isSel ? " sel" : "") + (inRange ? " inrange" : "") + (edited ? " edited" : "");
    // Field tint only shows when the byte isn't the active selection/highlight.
    const col = !isSel && !inRange && inFile ? colorAt?.(offset) : undefined;
    const tint = col ? { background: `color-mix(in srgb, ${col} 24%, transparent)` } : undefined;

    hexCells.push(
      <span
        key={i}
        className={"hex-byte" + cls + (i === 8 ? " gap" : "")}
        style={tint}
        onClick={inFile ? () => onSelect(offset) : undefined}
      >
        {!inFile ? "  " : b === undefined ? ".." : b.toString(16).padStart(2, "0").toUpperCase()}
      </span>
    );

    asciiCells.push(
      <span
        key={i}
        className={"ascii-char" + cls}
        style={tint}
        onClick={inFile ? () => onSelect(offset) : undefined}
      >
        {!inFile || b === undefined ? " " : b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : "."}
      </span>
    );
  }

  return (
    <div className="hex-row" style={{ top: row * ROW_HEIGHT }}>
      <span className="hex-offset">{base.toString(16).padStart(8, "0").toUpperCase()}</span>
      {mode === "hex" && <span className="hex-bytes">{hexCells}</span>}
      <span className={mode === "text" ? "hex-ascii text-wide" : "hex-ascii"}>{asciiCells}</span>
    </div>
  );
}
