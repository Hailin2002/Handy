import React from "react";

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  variant?: "default" | "compact";
}

export const Input: React.FC<InputProps> = ({
  className = "",
  variant = "default",
  disabled,
  ...props
}) => {
  const baseClasses =
    "px-2 py-1 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 rounded-md text-start transition-all duration-150";

  const interactiveClasses = disabled
    ? "opacity-60 cursor-not-allowed bg-mid-gray/10 border-mid-gray/40"
    : // Keyboard/mouse focus indicator: uniform focus-visible ring (inputs are
      // always :focus-visible while focused), replacing the native outline.
      "hover:bg-logo-primary/10 hover:border-logo-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:ring-offset-1 focus-visible:ring-offset-background focus:bg-logo-primary/20 focus:border-logo-primary";

  const variantClasses = {
    default: "min-h-[36px] px-3 py-2",
    compact: "min-h-[28px] px-2 py-1",
  } as const;

  return (
    <input
      className={`${baseClasses} ${variantClasses[variant]} ${interactiveClasses} ${className}`}
      disabled={disabled}
      {...props}
    />
  );
};
