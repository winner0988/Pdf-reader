import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import { defineConfig, globalIgnores } from "eslint/config";
import globals from "globals";
import tseslint from "typescript-eslint";

export default defineConfig([
  globalIgnores(["dist", "coverage", "src-tauri", "crates", "target"]),
  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat["recommended-latest"],
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      globals: globals.browser,
    },
    rules: {
      // AGENTS.md: the frontend never talks to the network (ADR 0009).
      "no-restricted-globals": [
        "error",
        ...["fetch", "XMLHttpRequest", "WebSocket", "EventSource"].map((name) => ({
          name,
          message: "The frontend must not make network requests (AGENTS.md, ADR 0009).",
        })),
      ],
      "no-restricted-properties": [
        "error",
        {
          object: "navigator",
          property: "sendBeacon",
          message: "The frontend must not make network requests (AGENTS.md, ADR 0009).",
        },
      ],
      // Strings shown in the UI will come from untrusted PDFs; never interpret them as HTML.
      "no-restricted-syntax": [
        "error",
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: "Do not render HTML; text from PDFs is untrusted (MVP-05).",
        },
      ],
    },
  },
  {
    // shadcn/ui components export their variant helpers next to the component.
    files: ["src/components/ui/**/*.tsx"],
    rules: {
      "react-refresh/only-export-components": "off",
    },
  },
]);
