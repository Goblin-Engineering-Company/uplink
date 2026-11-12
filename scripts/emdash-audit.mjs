// Em-dashes in USER-FACING copy under uplink/src/.
//
// Uses the TypeScript scanner rather than a regex, so the two agreed
// exclusions are applied mechanically and correctly:
//   * comment tokens (//, block, and JSX) are skipped: users never see them;
//   * the bare "—" / "— idle" empty-data placeholder is skipped: that is a
//     typographic convention for "no value", not prose.
// Everything else is prose a user can read. Exits 1 if any survives.
//
//   node emdash-audit.mjs              # working tree
//   node emdash-audit.mjs <git-rev>    # a revision, for a before/after count
import ts from "typescript";
import { readdirSync, statSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const ROOTS = ["src/views", "src/components", "src/lib/api.ts", "src/lib/types.ts",
               "src/App.tsx", "src/main.tsx"];
const PLACEHOLDER = new Set(["—", "— idle"]);
const rev = process.argv[2];

function walk(p, out = []) {
  if (statSync(p).isFile()) { if (/\.tsx?$/.test(p)) out.push(p); return out; }
  for (const e of readdirSync(p)) walk(join(p, e), out);
  return out;
}
const files = rev
  ? execFileSync("git", ["ls-tree", "-r", "--name-only", rev, "--",
      ...ROOTS], { encoding: "utf8" })
      .split("\n").filter((f) => /\.tsx?$/.test(f)).map((f) => f.replace(/^uplink\//, ""))
  : ROOTS.flatMap((r) => walk(r));

let n = 0;
for (const f of files) {
  const src = rev
    ? execFileSync("git", ["show", `${rev}:./${f}`], { encoding: "utf8", maxBuffer: 1 << 26 })
    : readFileSync(f, "utf8");
  const sf = ts.createSourceFile(f, src, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const seen = new Set();
  const visit = (node) => {
    // Only leaf-ish text carriers: string/template literals and JSX text.
    const kinds = [ts.SyntaxKind.StringLiteral, ts.SyntaxKind.JsxText,
                   ts.SyntaxKind.NoSubstitutionTemplateLiteral,
                   ts.SyntaxKind.TemplateHead, ts.SyntaxKind.TemplateMiddle,
                   ts.SyntaxKind.TemplateTail];
    if (kinds.includes(node.kind)) {
      const text = node.getText(sf);
      if (text.includes("—")) {
        const bare = text.replace(/^["'`]|["'`]$|^\}|\$\{$/g, "").trim();
        if (!PLACEHOLDER.has(bare)) {
          const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
          const key = `${f}:${line + 1}`;
          if (!seen.has(key)) {
            seen.add(key);
            console.log(`${key}: ${src.split("\n")[line].trim()}`);
            n++;
          }
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}
console.log(`\n${n} em-dash(es) in user-facing copy${rev ? ` at ${rev}` : ""}`);
process.exit(rev ? 0 : n > 0 ? 1 : 0);
