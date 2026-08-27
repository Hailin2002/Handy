import React from "react";
import { navItemClassName } from "./navItemClassName";
import type { ComponentType } from "react";
import type { NavIconProps } from "./productNav";

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
