import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "primary-soft"
    | "secondary"
    | "warning"
    | "danger"
    | "danger-ghost"
    | "ghost";
  size?: "sm" | "md" | "lg";
}

export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  variant = "primary",
  size = "md",
  ...props
}) => {
  // Keyboard focus indicator: a uniform focus-visible ring (offset so it stays
  // distinguishable from hover fills on every variant, incl. primary). The
  // native outline is suppressed only because the ring replaces it — there is
  // always a visible focus indicator.
  const baseClasses =
    "font-medium rounded-lg border transition-colors disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:ring-offset-2 focus-visible:ring-offset-background";

  const variantClasses = {
    primary:
      "text-white bg-background-ui border-background-ui hover:bg-background-ui/80 hover:border-background-ui/80",
    "primary-soft":
      "text-text bg-logo-primary/20 border-transparent hover:bg-logo-primary/30",
    secondary:
      "bg-mid-gray/10 border-mid-gray/20 hover:bg-background-ui/30 hover:border-logo-primary",
    // Secondary's neutral resting look, but hover uses the semantic
    // --color-warning token (theme.css) instead of the pink accent — for
    // buttons sitting on warning surfaces like SecureInputWarning
    warning:
      "text-text bg-mid-gray/10 border-mid-gray/20 hover:bg-warning/15 hover:border-warning",
    danger:
      "text-white bg-red-600 border-mid-gray/20 hover:bg-red-700 hover:border-red-700",
    "danger-ghost":
      "text-red-400 border-transparent hover:text-red-300 hover:bg-red-500/10",
    ghost:
      "text-current border-transparent hover:bg-mid-gray/10 hover:border-logo-primary",
  };

  // sm is reserved for dense toolbars: visually compact, but with an extended
  // invisible hit area (::after) so the effective touch target stays usable.
  // md and lg meet the >=44px minimum interactive size via min-height.
  const sizeClasses = {
    sm: "relative px-2 py-1 text-xs after:absolute after:-inset-1.5 after:content-['']",
    md: "min-h-[44px] px-4 py-[5px] text-sm",
    lg: "min-h-[48px] px-4 py-2 text-base",
  };

  return (
    <button
      className={`${baseClasses} ${variantClasses[variant]} ${sizeClasses[size]} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
};
