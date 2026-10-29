// Building blocks of the Settings tab: a titled group of rows, one row (text
// left, control right), a switch and a select. Native inputs stay underneath
// (keyboard, screen readers, tests); only their look is our own, so every
// control has the same height on Windows, macOS and Linux (app.css,
// "settings").

import { useId, type ChangeEvent, type ReactNode, type SelectHTMLAttributes } from "react";

export function Group({ title, children, footer, tone }: { title: string; children: ReactNode; footer?: ReactNode; tone?: "danger" }) {
  const id = useId();
  return (
    <section className={tone ? `group group-${tone}` : "group"} aria-labelledby={id}>
      <h2 className="group-title" id={id}>
        {title}
      </h2>
      <div className="group-body">{children}</div>
      {footer && <p className="group-foot">{footer}</p>}
    </section>
  );
}

interface RowProps {
  title: ReactNode;
  /** Shown under the title; also the control's description for screen readers. */
  desc?: ReactNode;
  descId?: string;
  descClass?: string;
  children?: ReactNode;
  /** Long controls (lists, sliders on narrow windows) go under the text. */
  stacked?: boolean;
}

export function Row({ title, desc, descId, descClass, children, stacked = false }: RowProps) {
  return (
    <div className={stacked ? "row is-stacked" : "row"}>
      <div className="row-text">
        <div className="row-title">{title}</div>
        {desc && (
          <div className={descClass ? `row-desc ${descClass}` : "row-desc"} id={descId}>
            {desc}
          </div>
        )}
      </div>
      {children && <div className="row-control">{children}</div>}
    </div>
  );
}

interface ToggleProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  describedBy?: string;
}

/** On/off switch: a real checkbox (role "switch") under a drawn track. */
export function Toggle({ label, checked, onChange, disabled, describedBy }: ToggleProps) {
  return (
    <span className="toggle">
      <input
        type="checkbox"
        role="switch"
        aria-label={label}
        aria-describedby={describedBy}
        checked={checked}
        disabled={disabled}
        onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(e.target.checked)}
      />
      <span className="toggle-track" aria-hidden="true" />
    </span>
  );
}

/** A row whose whole text toggles the switch on the right. */
export function ToggleRow({
  title,
  desc,
  descClass,
  ...toggle
}: Omit<ToggleProps, "label" | "describedBy"> & { title: string; desc?: ReactNode; descClass?: string }) {
  const descId = useId();
  return (
    <label className={toggle.disabled ? "row is-toggle is-disabled" : "row is-toggle"}>
      <span className="row-text">
        <span className="row-title">{title}</span>
        {desc && (
          <span className={descClass ? `row-desc ${descClass}` : "row-desc"} id={descId}>
            {desc}
          </span>
        )}
      </span>
      <span className="row-control">
        <Toggle label={title} describedBy={desc ? descId : undefined} {...toggle} />
      </span>
    </label>
  );
}

/** Native select without the platform chrome, with our own chevron. */
export function Select({ className, children, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <span className={className ? `select ${className}` : "select"}>
      <select {...props}>{children}</select>
      <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true" focusable="false">
        <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </span>
  );
}
