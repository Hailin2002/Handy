import React from "react";

type IconButtonProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "aria-label"
> & {
  /**
   * Mandatory accessible name for the icon action. TypeScript forbids
   * rendering an IconButton without it — never rely on `title` alone.
   */
  "aria-label": string;
  /** Highlights the button as toggled on (e.g. a saved entry). */
  active?: boolean;
};

/**
 * Shared icon-only button primitive (a11y baseline D2).
 *
 * Enforces:
 * - a required, typed `aria-label` accessible name,
 * - a >=44x44 effective hit area (`min-h-11 min-w-11`; compact icons stay
 *   centered inside the enlarged target),
 * - a standard visible `focus-visible` ring.
 *
 * Color/visual variants are passed via `className` by call sites.
 */
export const IconButton: React.FC<IconButtonProps> = ({
  active = false,
  className = "",
  type = "button",
  children,
  ...props
}) => (
  <button
    type={type}
    {...props}
    className={`inline-flex min-h-11 min-w-11 shrink-0 items-center justify-center rounded-md border border-transparent p-2 transition-colors cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-not-allowed disabled:text-text/20 disabled:opacity-50 ${
      active
        ? "text-logo-primary hover:text-logo-primary/80"
        : "text-text/60 hover:bg-logo-primary/10 hover:text-logo-primary"
    } ${className}`}
  >
    {children}
  </button>
);
