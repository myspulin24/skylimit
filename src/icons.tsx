// Small inline icons shared by the widget and the app window. Geometry lives in
// attributes (the CSP forbids inline style in HTML, not SVG attributes).

/** Speech bubble: feedback ("Zpětná vazba"). */
export function FeedbackIcon() {
  return (
    <svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true" focusable="false">
      <path
        d="M4.2 3.8h11.6a1.9 1.9 0 0 1 1.9 1.9v6.9a1.9 1.9 0 0 1-1.9 1.9H9.6l-3.9 3v-3H4.2a1.9 1.9 0 0 1-1.9-1.9V5.7a1.9 1.9 0 0 1 1.9-1.9Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
      <g fill="currentColor">
        <circle cx="6.7" cy="9.2" r="1.05" />
        <circle cx="10" cy="9.2" r="1.05" />
        <circle cx="13.3" cy="9.2" r="1.05" />
      </g>
    </svg>
  );
}
