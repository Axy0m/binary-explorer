// A Monaco-based editor for the schema DSL, replacing the plain <textarea>.
//
// This app runs offline inside a Tauri WebView with a null CSP, so Monaco must
// be *bundled* — we cannot use @monaco-editor/react's default CDN loader. We
// import the `monaco-editor` package directly, hand it to the loader via
// `loader.config({ monaco })`, and wire up the editor web worker through Vite's
// `?worker` import so no network fetch ever happens.
import { useEffect, useRef } from "react";
import Editor, { loader, type Monaco, type OnMount } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
// Vite bundles this worker and returns a constructor we can `new`.
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

// Serve Monaco from our own bundle instead of a CDN.
loader.config({ monaco });

// The schema DSL only needs the plain editor worker (no TS/JSON/CSS language
// services), so route every worker request to it.
self.MonacoEnvironment = {
  getWorker() {
    return new EditorWorker();
  },
};

const LANG_ID = "bxschema";
const THEME_ID = "nybble-dark";

let registered = false;

/** Register the DSL language + theme once, before the first editor mounts. */
function registerLanguage(m: Monaco) {
  if (registered) return;
  registered = true;

  m.languages.register({ id: LANG_ID });

  m.languages.setLanguageConfiguration(LANG_ID, {
    comments: { lineComment: "//" },
    brackets: [
      ["{", "}"],
      ["[", "]"],
      ["(", ")"],
    ],
    autoClosingPairs: [
      { open: "{", close: "}" },
      { open: "[", close: "]" },
      { open: "(", close: ")" },
      { open: '"', close: '"' },
    ],
    surroundingPairs: [
      { open: "{", close: "}" },
      { open: "[", close: "]" },
      { open: "(", close: ")" },
      { open: '"', close: '"' },
    ],
  });

  m.languages.setMonarchTokensProvider(LANG_ID, {
    keywords: [
      "struct", "enum", "bitfield", "if", "at", "match", "default",
    ],
    typeKeywords: [
      "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64",
      "f32", "f64", "bool", "char", "string", "bytes", "cstring",
    ],
    operators: [
      "=>", "==", "!=", "<=", ">=", "<", ">", "=",
      "+", "-", "*", "/", "%", ":", ".",
    ],
    symbols: /[=><!~?:&|+\-*/^%.]+/,
    tokenizer: {
      root: [
        // metadata header comments: `// @name Foo`
        [/\/\/\s*@\w+.*$/, "comment.doc"],
        [/\/\/.*$/, "comment"],
        [/"([^"\\]|\\.)*"/, "string"],
        [/"([^"\\]|\\.)*$/, "string.invalid"],
        [/0[xX][0-9a-fA-F]+/, "number.hex"],
        [/\d+/, "number"],
        [
          /[a-zA-Z_]\w*/,
          {
            cases: {
              "@keywords": "keyword",
              "@typeKeywords": "type",
              "@default": "identifier",
            },
          },
        ],
        [/[{}()[\]]/, "@brackets"],
        [
          /@symbols/,
          { cases: { "@operators": "operator", "@default": "" } },
        ],
        [/[ \t\r\n]+/, "white"],
      ],
    },
  });

  m.editor.defineTheme(THEME_ID, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "comment", foreground: "56697f", fontStyle: "italic" },
      { token: "comment.doc", foreground: "7b8ea6", fontStyle: "italic" },
      { token: "keyword", foreground: "f472b6" },
      { token: "type", foreground: "a78bfa" },
      { token: "number", foreground: "fbbf24" },
      { token: "number.hex", foreground: "fbbf24" },
      { token: "string", foreground: "34d399" },
      { token: "string.invalid", foreground: "e23d4d" },
      { token: "operator", foreground: "5eead4" },
      { token: "identifier", foreground: "e3edf8" },
    ],
    colors: {
      "editor.background": "#0b0f14",
      "editor.foreground": "#e3edf8",
      "editorLineNumber.foreground": "#3a4a60",
      "editorLineNumber.activeForeground": "#7b8ea6",
      "editor.selectionBackground": "#1b4b47",
      "editor.lineHighlightBackground": "#111823",
      "editorCursor.foreground": "#2dd4bf",
      "editorIndentGuide.background1": "#1b2634",
      "editorError.foreground": "#e23d4d",
    },
  });
}

/** Pull a 1-based (line, col) out of a parse-error message, if present.
 *  Every ParseError except UnexpectedEof carries "line N, col N". */
function markerFromError(
  m: Monaco,
  model: monaco.editor.ITextModel,
  message: string,
): monaco.editor.IMarkerData {
  const match = /line (\d+), col (\d+)/.exec(message);
  const line = match ? Number(match[1]) : 1;
  // UnexpectedEof has no position — point at the end of the last line.
  const col = match ? Number(match[2]) : model.getLineMaxColumn(model.getLineCount());
  return {
    severity: m.MarkerSeverity.Error,
    message,
    startLineNumber: line,
    startColumn: col,
    endLineNumber: line,
    endColumn: col + 1,
  };
}

interface Props {
  value: string;
  onChange: (text: string) => void;
  /** Latest parse error text, or empty when the schema parsed cleanly. */
  error?: string;
}

export function SchemaEditor({ value, onChange, error }: Props) {
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);

  const handleMount: OnMount = (editor, m) => {
    editorRef.current = editor;
    monacoRef.current = m;
    syncMarkers(m, editor.getModel(), error);
  };

  // Reflect the current parse error as an inline squiggle whenever it changes.
  useEffect(() => {
    const m = monacoRef.current;
    const editor = editorRef.current;
    if (!m || !editor) return;
    syncMarkers(m, editor.getModel(), error);
  }, [error]);

  return (
    <div className="schema-editor-monaco">
      <Editor
        language={LANG_ID}
        theme={THEME_ID}
        value={value}
        beforeMount={registerLanguage}
        onMount={handleMount}
        onChange={(v) => onChange(v ?? "")}
        options={{
          fontFamily: 'var(--mono), "JetBrains Mono", "Cascadia Code", Consolas, monospace',
          fontSize: 12,
          lineHeight: 18,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          lineNumbers: "on",
          renderLineHighlight: "line",
          folding: false,
          tabSize: 2,
          automaticLayout: true,
          scrollbar: { verticalScrollbarSize: 8, horizontalScrollbarSize: 8 },
          overviewRulerLanes: 0,
          padding: { top: 6, bottom: 6 },
        }}
      />
    </div>
  );
}

function syncMarkers(
  m: Monaco,
  model: monaco.editor.ITextModel | null,
  error?: string,
) {
  if (!model) return;
  const markers = error ? [markerFromError(m, model, error)] : [];
  m.editor.setModelMarkers(model, "bxschema", markers);
}
