interface Props {
  /** Normalized entropy values in [0,1], one per bucket across the file. */
  data: number[];
  fileLen: number;
  onSeek: (offset: number) => void;
}

/** Color a bar by its entropy: low = teal (structured), high = pink (packed). */
function barColor(v: number): string {
  if (v < 0.4) return "var(--accent)"; // teal — low/structured
  if (v < 0.75) return "var(--amber)"; // amber — mixed
  return "var(--pink)"; // pink — high/compressed/encrypted
}

/**
 * "Byte entropy across file" — a strip of bars showing how random each region
 * of the file is. Spikes reveal compressed/encrypted blocks; click to jump.
 */
export function EntropyStrip({ data, fileLen, onSeek }: Props) {
  if (data.length === 0) return null;
  return (
    <div className="entropy">
      {data.map((v, i) => (
        <div
          key={i}
          className="entropy-bar"
          style={{ height: `${Math.max(3, v * 100)}%`, background: barColor(v) }}
          title={`~0x${Math.floor((i / data.length) * fileLen).toString(16)} · entropy ${(v * 100).toFixed(0)}%`}
          onClick={() => onSeek(Math.floor((i / data.length) * fileLen))}
        />
      ))}
    </div>
  );
}
