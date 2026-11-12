// ESLint flat config for the Uplink frontend.
//
// Why this exists: `tsc --noEmit` was the project's ONLY frontend gate, and three
// files already carried `// eslint-disable-next-line react-hooks/exhaustive-deps`
// comments for a rule that had never actually run. The hooks rules below are the
// point of this config — they catch the missing/wrong dependency arrays that let
// an effect re-fire on every render (see the 2026-08-08 Uplink refactor audit,
// findings H11 and H12).
//
// Type-aware linting (typescript-eslint's `recommendedTypeChecked`) is deliberately
// NOT enabled: it needs a full type graph per run and would make the gate slow
// enough that it stops getting run. `tsc --noEmit` already covers type errors.

import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  {
    // Build output, deps, and the Rust tree are not ours to lint.
    ignores: ["dist/**", "node_modules/**", "src-tauri/**"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: globals.browser,
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // The whole reason for the config — a missing dep array is a real bug here,
      // not a style preference.
      "react-hooks/exhaustive-deps": "error",
      // `_`-prefixed args/vars are an intentional "unused, and I mean it" marker.
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  {
    // Node context: config files run outside the browser.
    files: ["*.config.{js,ts}", "gen-icon.mjs", "scripts/**/*.mjs"],
    languageOptions: { globals: globals.node },
  },
);
