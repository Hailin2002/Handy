import React from "react";
import { useTranslation } from "react-i18next";
import ResetIcon from "../icons/ResetIcon";
import { IconButton } from "./IconButton";

interface ResetButtonProps {
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  /** Accessible name; defaults to the localized generic "Reset". */
  ariaLabel?: string;
  children?: React.ReactNode;
}

export const ResetButton: React.FC<ResetButtonProps> = React.memo(
  ({ onClick, disabled = false, className = "", ariaLabel, children }) => {
    const { t } = useTranslation();

    return (
      <IconButton
        onClick={onClick}
        disabled={disabled}
        aria-label={ariaLabel ?? t("common.reset")}
        className={className}
      >
        {children ?? <ResetIcon />}
      </IconButton>
    );
  },
);
