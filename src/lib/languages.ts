import { StreamLanguage, type LanguageSupport } from "@codemirror/language";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import type { Extension } from "@codemirror/state";

/** Langages colorés par l'éditeur ; tout le reste est du texte brut. */
export type EditorLanguageId =
  | "rust"
  | "typescript"
  | "tsx"
  | "javascript"
  | "jsx"
  | "python"
  | "json"
  | "toml"
  | "yaml"
  | "markdown"
  | "css"
  | "html"
  | "plain";

/** Déduit le langage d'un chemin à partir de son extension (casse ignorée). */
export function languageIdForPath(path: string): EditorLanguageId {
  const clean = path.split("?")[0]?.split("#")[0] ?? path;
  const dot = clean.lastIndexOf(".");
  const extension = dot >= 0 ? clean.slice(dot + 1).toLowerCase() : "";
  switch (extension) {
    case "rs":
      return "rust";
    case "ts":
      return "typescript";
    case "tsx":
      return "tsx";
    case "js":
    case "mjs":
    case "cjs":
      return "javascript";
    case "jsx":
      return "jsx";
    case "py":
    case "pyi":
      return "python";
    case "json":
    case "jsonc":
      return "json";
    case "toml":
      return "toml";
    case "yaml":
    case "yml":
      return "yaml";
    case "md":
    case "markdown":
    case "mdx":
      return "markdown";
    case "css":
    case "scss":
    case "less":
      return "css";
    case "html":
    case "htm":
      return "html";
    default:
      return "plain";
  }
}

/** Extension CodeMirror 6 correspondant au langage, sans dépendance dynamique. */
export function languageExtension(id: EditorLanguageId): Extension {
  switch (id) {
    case "rust":
      return rust();
    case "typescript":
      return javascript({ typescript: true });
    case "tsx":
      return javascript({ typescript: true, jsx: true });
    case "javascript":
      return javascript();
    case "jsx":
      return javascript({ jsx: true });
    case "python":
      return python();
    case "json":
      return json();
    case "toml":
      return StreamLanguage.define(toml);
    case "yaml":
      return yaml();
    case "markdown":
      return markdown();
    case "css":
      return css();
    case "html":
      return html();
    default:
      return [];
  }
}

/** Indication lisible (coloration appliquée) pour la barre de l'éditeur. */
export const LANGUAGE_LABELS: Record<EditorLanguageId, string> = {
  rust: "Rust",
  typescript: "TypeScript",
  tsx: "TSX",
  javascript: "JavaScript",
  jsx: "JSX",
  python: "Python",
  json: "JSON",
  toml: "TOML",
  yaml: "YAML",
  markdown: "Markdown",
  css: "CSS",
  html: "HTML",
  plain: "Texte",
};

export type LanguageSupportExtension = LanguageSupport | Extension;
