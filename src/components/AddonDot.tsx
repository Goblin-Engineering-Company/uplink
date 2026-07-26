import { addonAccentVar } from "../lib/themes";

// The color dot that identifies an addon everywhere (tray lines + accordions).
export function AddonDot({ slug, size = 10 }: { slug: string; size?: number }) {
  return (
    <span
      className="dot"
      style={{ width: size, height: size, background: addonAccentVar(slug), boxShadow: `0 0 6px ${addonAccentVar(slug)}` }}
    />
  );
}
