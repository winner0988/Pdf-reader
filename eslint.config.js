import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import { defineConfig, globalIgnores } from "eslint/config";
import globals from "globals";
import tseslint from "typescript-eslint";

const noChineseInCode = {
  meta: {
    type: "problem",
    docs: {
      description: "Disallow Chinese characters in string literals and JSX text outside i18n and tests (ADR 0029).",
    },
    schema: [],
    messages: {
      noChinese: "Hardcoded Chinese character found in code: '{{text}}'. Move user-facing strings to src/i18n (ADR 0029).",
    },
  },
  create(context) {
    const filename = (context.filename ?? context.getFilename?.() ?? "").replace(/\\/g, "/");
    if (
      filename.includes("/src/i18n/") ||
      filename.includes(".test.") ||
      filename.includes(".spec.") ||
      filename.includes("/src/test/") ||
      filename.endsWith("/src/features/shell/demo.ts")
    ) {
      return {};
    }
    const CHINESE_REGEX = /[\u4e00-\u9fa5]/;
    return {
      Literal(node) {
        if (typeof node.value === "string" && CHINESE_REGEX.test(node.value)) {
          context.report({
            node,
            messageId: "noChinese",
            data: { text: node.value },
          });
        }
      },
      JSXText(node) {
        if (CHINESE_REGEX.test(node.value)) {
          context.report({
            node,
            messageId: "noChinese",
            data: { text: node.value.trim() },
          });
        }
      },
      TemplateElement(node) {
        if (CHINESE_REGEX.test(node.value.raw)) {
          context.report({
            node,
            messageId: "noChinese",
            data: { text: node.value.raw.trim() },
          });
        }
      },
    };
  },
};

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
  {
    // User-facing strings must be in src/i18n; forbid hardcoded Chinese in code (ADR 0029).
    files: ["src/**/*.{ts,tsx}"],
    plugins: {
      custom: {
        rules: {
          "no-chinese": noChineseInCode,
        },
      },
    },
    rules: {
      "custom/no-chinese": "error",
    },
  },
]);
