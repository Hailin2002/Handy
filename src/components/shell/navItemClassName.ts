// Shared nav-item styling.
//
// Lives beside NavItem rather than inside it so that file exports only its
// component: react-refresh cannot hot-reload a module that also exports
// values, and this string builder is used by SettingsPage too.

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
