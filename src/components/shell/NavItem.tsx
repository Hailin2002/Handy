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
    {/* Wraps rather than truncating: the sidebar is 160px, and "Dictation
        History" (17 chars) already does not fit in English — French is 22
        ("Historique des dictées") and Portuguese 20. A primary navigation
        label that can only be read by hovering for the title tooltip is not
        readable. `min-h-[44px]` on the button absorbs the second line. */}
    <span className="text-sm font-medium leading-tight text-start">
      {label}
    </span>
  </button>
);
