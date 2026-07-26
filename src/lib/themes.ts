// The theme registry. Adding a new theme = one CSS block in styles/themes.css
// plus one entry here (and it appears in the Settings picker automatically).
export const THEMES: Array<{ id: string; name: string; blurb: string }> = [
  { id: "workshop", name: "Workshop", blurb: "Brass & navy machine hall (default)" },
  { id: "blueprint", name: "Blueprint", blurb: "Drafting-table steel blues & cyan ink" },
  { id: "parchment", name: "Parchment", blurb: "Warm light ledger with ember accents" },
];

export function applyTheme(theme: string) {
  document.documentElement.setAttribute("data-theme", theme || "workshop");
}

// per-addon accent color, resolved from the CSS variable set in themes.css.
export function addonAccentVar(slug: string): string {
  const key = (slug || "").toLowerCase().replace(/[^a-z]/g, "");
  const known: Record<string, string> = {
    sbf: "--addon-sbf",
    haul: "--addon-haul",
    megaphone: "--addon-megaphone",
    recall: "--addon-recall",
  };
  return `var(${known[key] ?? "--addon-default"})`;
}
