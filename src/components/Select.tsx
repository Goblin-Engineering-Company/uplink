import { useEffect, useRef, useState } from "react";

export type SelectOption = { value: string; label: string };

// A fully CSS-controlled dropdown. Native <select>/<option> popups can't be
// themed on older webkit (SteamOS/Steam Deck renders them white-on-white and
// unreadable); this replaces them so the control looks identical everywhere.
// Closes on outside-click and Escape.
export function Select({
  value,
  options,
  onChange,
  disabled,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = options.find((o) => o.value === value);

  return (
    <div className={`jsel${open ? " open" : ""}`} ref={ref}>
      <button
        type="button"
        className="jsel-btn"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="jsel-label">{current?.label ?? ""}</span>
        <span className="jsel-caret">▾</span>
      </button>
      {open && (
        <ul className="jsel-menu" role="listbox">
          {options.map((o) => (
            <li
              key={o.value}
              role="option"
              aria-selected={o.value === value}
              className={`jsel-opt${o.value === value ? " sel" : ""}`}
              onClick={() => { onChange(o.value); setOpen(false); }}
            >
              {o.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
