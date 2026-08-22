import React from "react";
import type { ComponentType } from "react";
import type { NavIconProps } from "./productNav";

/**
 * Shared navigation-button styling, built on the button/nav semantics
 * established for the sidebar (keyboard-operable <button>, aria-current,
 * visible focus ring, 44px minimum touch target).
 */
export const navItemClassName = (active: boolean): string =>
  `flex gap-2 items-center px-2 w-full min-h-[44px] rounded-lg cursor-pointer transition-colors outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
    active
      ? "bg-logo-primary/80"
      : "hover:bg-mid-gray/20 hover:opacity-100 opacity-85"
  }`;

interface NavItemProps {
  label: string;
  icon: ComponentType<NavIconProps>;
  active: boolean;
  onClick: () => void;
}

export const NavItem: React.FC<NavItemProps> = ({
  label,
  icon: Icon,
  active,
  onClick,
}) => (
  <button
    type="button"
    onClick={onClick}
    aria-current={active ? "page" : undefined}
    title={label}
    className={navItemClassName(active)}
  >
    <Icon width={24} height={24} className="shrink-0" />
    <span className="text-sm font-medium truncate">{label}</span>
  </button>
);
