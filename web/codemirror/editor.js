// The shader editor: CodeMirror, configured for the app. Bundled into
// codemirror.js (`npm install && npm run build` here), which defines the
// global `OngakueCodeMirror` used by the Rust side (crates/app/src/editor/code.rs).
//
// What to suggest, explain on hover, and which words to color all come from
// the Rust side, through the `hooks` given to `create`.

import { basicSetup } from "codemirror";
import { EditorState, Compartment, Prec } from "@codemirror/state";
import { EditorView, keymap, hoverTooltip } from "@codemirror/view";
import { indentWithTab } from "@codemirror/commands";
import { StreamLanguage, HighlightStyle, syntaxHighlighting, indentUnit } from "@codemirror/language";
import { autocompletion } from "@codemirror/autocomplete";
import { setDiagnostics, lintGutter } from "@codemirror/lint";
import { tags } from "@lezer/highlight";

// A tokenizer for one language, from its word lists. Like the shader
// languages, without strings: comments, numbers, attributes or preprocessor
// directives (`@fragment`, `#define`), `u.field`, and words.
function shaderLanguage(name, words) {
  const sets = Object.fromEntries(Object.entries(words).map(([kind, list]) => [kind, new Set(list)]));
  return StreamLanguage.define({
    name,
    startState: () => ({ inComment: false }),
    token(stream, state) {
      if (state.inComment) {
        if (stream.skipTo("*/")) {
          stream.pos += 2;
          state.inComment = false;
        } else {
          stream.skipToEnd();
        }
        return "comment";
      }
      if (stream.eatSpace()) return null;
      if (stream.match("//")) {
        stream.skipToEnd();
        return "comment";
      }
      if (stream.match("/*")) {
        state.inComment = true;
        return "comment";
      }
      if (stream.match(/^(0x[0-9a-fA-F]+|(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?)[uifh]?/)) return "number";
      if (stream.match(/^[@#][A-Za-z_]\w*/)) return "meta";
      if (stream.match(/^u\.[A-Za-z_]\w*/)) return "input";
      const word = stream.match(/^[A-Za-z_]\w*/);
      if (word) {
        const text = word[0];
        if (sets.keywords.has(text)) return "keyword";
        if (sets.types.has(text)) return "typeName";
        if (sets.functions.has(text)) return "builtinFunction";
        if (sets.inputs.has(text)) return "input";
        return null;
      }
      stream.next();
      return null;
    },
    languageData: { commentTokens: { line: "//", block: { open: "/*", close: "*/" } } },
    tokenTable: {
      builtinFunction: tags.standard(tags.function(tags.variableName)),
      input: tags.special(tags.variableName),
    },
  });
}

// The editor's colors, matching the rest of the app.
const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#ee3a67" },
  { tag: tags.typeName, color: "#69dce9" },
  { tag: tags.standard(tags.function(tags.variableName)), color: "#8fd647" },
  { tag: tags.special(tags.variableName), color: "#f3d98c" },
  { tag: tags.number, color: "#ab79f1" },
  { tag: tags.meta, color: "#fc9867" },
  { tag: tags.comment, color: "#6f7f8f", fontStyle: "italic" },
]);

const theme = EditorView.theme(
  {
    "&": { height: "100%", color: "#e6e6e6", backgroundColor: "transparent", fontSize: "13px" },
    ".cm-scroller": { fontFamily: "ui-monospace, Consolas, monospace", lineHeight: "1.45" },
    ".cm-content": { caretColor: "#fff" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#fff" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "rgba(120, 160, 255, 0.35) !important",
    },
    ".cm-gutters": { backgroundColor: "transparent", color: "#505868", borderRight: "1px solid #2a2e38" },
    ".cm-activeLine": { backgroundColor: "rgba(255, 255, 255, 0.04)" },
    ".cm-activeLineGutter": { backgroundColor: "transparent", color: "#9aa3b5" },
    ".cm-matchingBracket": { backgroundColor: "rgba(255, 136, 0, 0.25)", outline: "none" },
    ".cm-tooltip": {
      backgroundColor: "#1b1d26",
      color: "#ddd",
      border: "1px solid #3a3f4b",
      borderRadius: "4px",
      boxShadow: "0 4px 12px rgba(0, 0, 0, 0.5)",
    },
    ".cm-tooltip-autocomplete ul li[aria-selected]": { backgroundColor: "rgba(120, 160, 255, 0.3)", color: "#fff" },
    ".cm-completionDetail": { color: "#8a93a3", fontStyle: "normal", marginLeft: "1em" },
    ".cm-completionInfo, .cm-hover-doc": {
      maxWidth: "420px",
      padding: "4px 8px",
      font: "12px/1.4 system-ui, sans-serif",
      color: "#bbb",
    },
    ".cm-hover-title": { padding: "4px 8px 0", font: "12px ui-monospace, Consolas, monospace", color: "#eee" },
    ".cm-panels": { backgroundColor: "#1b1d26", color: "#ddd" },
    ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy #ff6b6b" },
  },
  { dark: true },
);

// The CodeMirror view inside `element`, e.g. to inspect it from the
// browser's console or a test: `OngakueCodeMirror.viewOf(document.querySelector(".cm-editor"))`.
export function viewOf(element) {
  return EditorView.findFromDOM(element);
}

// `parent`: the element to fill. `hooks`: `changed(code)` after each edit,
// `compile()` on Ctrl+Enter (Cmd+Enter on a Mac),
// `complete(code, position)` and `hover(code, position)` (positions in UTF-16
// units, like JavaScript strings), and `words`: per language, the lists of
// keywords, types, functions and inputs to color.
export function create(parent, hooks) {
  const languages = {
    wgsl: shaderLanguage("wgsl", hooks.words.wgsl),
    glsl: shaderLanguage("glsl", hooks.words.glsl),
  };
  const language = new Compartment();
  let current = "wgsl";

  // Suggestions are ranked by the Rust side: keep its order.
  const complete = (context) => {
    const result = hooks.complete(context.state.doc.toString(), context.pos);
    return result ? { from: result.from, options: result.options, filter: false } : null;
  };
  const hover = hoverTooltip((view, position) => {
    const result = hooks.hover(view.state.doc.toString(), position);
    if (!result) return null;
    return {
      pos: result.from,
      above: true,
      create() {
        const dom = document.createElement("div");
        const title = dom.appendChild(document.createElement("div"));
        title.className = "cm-hover-title";
        title.textContent = result.title;
        const doc = dom.appendChild(document.createElement("div"));
        doc.className = "cm-hover-doc";
        doc.textContent = result.doc;
        return { dom };
      },
    };
  });

  const extensions = () => [
    basicSetup,
    keymap.of([indentWithTab]),
    // Before the default keymap, where Ctrl+Enter inserts a line.
    Prec.highest(keymap.of([{ key: "Mod-Enter", run: () => (hooks.compile(), true) }])),
    indentUnit.of("    "),
    EditorState.tabSize.of(4),
    language.of(languages[current]),
    syntaxHighlighting(highlight),
    theme,
    autocompletion({ override: [complete] }),
    hover,
    lintGutter(),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) hooks.changed(update.state.doc.toString());
    }),
  ];
  const view = new EditorView({ parent, state: EditorState.create({ doc: "", extensions: extensions() }) });

  return {
    getValue: () => view.state.doc.toString(),
    // Shows another visual's code, with its own undo history.
    setValue(code) {
      view.setState(EditorState.create({ doc: code, extensions: extensions() }));
    },
    // "wgsl" or "glsl": how to color the code.
    setLanguage(name) {
      if (name === current || !languages[name]) return;
      current = name;
      view.dispatch({ effects: language.reconfigure(languages[name]) });
    },
    // Marks the error's line (from 1), or clears the errors with line 0.
    setError(line, message) {
      const doc = view.state.doc;
      const diagnostics =
        line >= 1 && line <= doc.lines
          ? [{ from: doc.line(line).from, to: doc.line(line).to, severity: "error", message }]
          : [];
      view.dispatch(setDiagnostics(view.state, diagnostics));
    },
    focus: () => view.focus(),
  };
}
