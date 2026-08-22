import React, { useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "./IconButton";
import { Tooltip } from "./Tooltip";

interface SettingContainerProps {
  title: string;
  description: string;
  children: React.ReactNode;
  /**
   * "inline" (default) keeps the description visible without hover so the
   * setting is self-explanatory; "tooltip" hides it behind an info button.
   */
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  layout?: "horizontal" | "stacked";
  disabled?: boolean;
  tooltipPosition?: "top" | "bottom";
}

export const SettingContainer: React.FC<SettingContainerProps> = ({
  title,
  description,
  children,
  descriptionMode = "inline",
  grouped = false,
  layout = "horizontal",
  disabled = false,
  tooltipPosition = "top",
}) => {
  // Stable id shared between the trigger's `aria-describedby` and the
  // tooltip node, so screen readers announce the description when the
  // info button is focused or hovered.
  const tooltipId = useId();
  const tooltipRef = useRef<HTMLDivElement>(null);
  const { t } = useTranslation();
  const infoLabel = t("common.moreInformation");

  const containerClasses = grouped
    ? "px-4 p-2"
    : "px-4 p-2 rounded-lg border border-mid-gray/20";

  if (layout === "stacked") {
    if (descriptionMode === "tooltip") {
      return (
        <div className={containerClasses}>
          <div className="flex items-center gap-2 mb-2">
            <h3
              className={`text-sm font-medium ${disabled ? "opacity-50" : ""}`}
            >
              {title}
            </h3>
            <div ref={tooltipRef} className="relative">
              <IconButton
                aria-label={infoLabel}
                aria-describedby={tooltipId}
                className="text-mid-gray hover:bg-transparent hover:text-logo-primary"
              >
                <svg
                  className="w-4 h-4 select-none"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
              </IconButton>
              <Tooltip targetRef={tooltipRef} id={tooltipId} position="top">
                <p className="text-sm text-center leading-relaxed">
                  {description}
                </p>
              </Tooltip>
            </div>
          </div>
          <div className="w-full">{children}</div>
        </div>
      );
    }

    return (
      <div className={containerClasses}>
        <div className="mb-2">
          <h3 className={`text-sm font-medium ${disabled ? "opacity-50" : ""}`}>
            {title}
          </h3>
          <p className={`text-sm ${disabled ? "opacity-50" : ""}`}>
            {description}
          </p>
        </div>
        <div className="w-full">{children}</div>
      </div>
    );
  }

  // Horizontal layout (default)
  const horizontalContainerClasses = grouped
    ? "flex items-center justify-between min-h-12 px-4 p-2"
    : "flex items-center justify-between min-h-12 px-4 p-2 rounded-lg border border-mid-gray/20";

  if (descriptionMode === "tooltip") {
    return (
      <div className={horizontalContainerClasses}>
        <div className="max-w-2/3">
          <div className="flex items-center gap-2">
            <h3
              className={`text-sm font-medium ${disabled ? "opacity-50" : ""}`}
            >
              {title}
            </h3>
            <div ref={tooltipRef} className="relative">
              <IconButton
                aria-label={infoLabel}
                aria-describedby={tooltipId}
                className="text-mid-gray hover:bg-transparent hover:text-logo-primary"
              >
                <svg
                  className="w-4 h-4 select-none"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
              </IconButton>
              <Tooltip
                targetRef={tooltipRef}
                id={tooltipId}
                position={tooltipPosition}
              >
                <p className="text-sm text-center leading-relaxed">
                  {description}
                </p>
              </Tooltip>
            </div>
          </div>
        </div>
        <div className="relative">{children}</div>
      </div>
    );
  }

  return (
    <div className={horizontalContainerClasses}>
      <div className="max-w-2/3">
        <h3 className={`text-sm font-medium ${disabled ? "opacity-50" : ""}`}>
          {title}
        </h3>
        <p className={`text-sm ${disabled ? "opacity-50" : ""}`}>
          {description}
        </p>
      </div>
      <div className="relative">{children}</div>
    </div>
  );
};
