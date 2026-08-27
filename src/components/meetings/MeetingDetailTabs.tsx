import React, { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  MEETING_DETAIL_TABS,
  edgeTab,
  selectableTabs,
  stepTab,
  tabLabelKey,
  type MeetingDetailTab,
} from "./meetingDetailTabModel";

interface MeetingDetailTabsProps {
  active: MeetingDetailTab;
  onChange: (tab: MeetingDetailTab) => void;
  /** Tabs to render as present but not selectable, with a reason in the title. */
  disabled?: Partial<Record<MeetingDetailTab, string>>;
}

/**
 * Tablist for the meeting detail view (plan §6.13).
 *
 * The plan opens on `Übersicht` and keeps the rest one click away rather than
 * stacking everything into one scroll; it also requires full keyboard
 * operation with a visible focus ring (§6.14), so this implements the standard
 * roving-tabindex pattern: arrows move selection, Home/End jump to the ends,
 * and only the active tab is in the tab order.
 *
 * A tab that has nothing to show (no audio left after a retention deletion)
 * stays visible but disabled with an explanation, instead of disappearing and
 * leaving the user wondering where it went.
 */
export const MeetingDetailTabs: React.FC<MeetingDetailTabsProps> = ({
  active,
  onChange,
  disabled = {},
}) => {
  const { t } = useTranslation();
  const refs = useRef(new Map<MeetingDetailTab, HTMLButtonElement | null>());

  const selectable = selectableTabs(disabled);

  const go = useCallback(
    (next: MeetingDetailTab | null) => {
      if (!next) return;
      onChange(next);
      refs.current.get(next)?.focus();
    },
    [onChange],
  );

  // If the active tab becomes unavailable (its data was deleted), fall back to
  // the first selectable one rather than rendering an empty panel.
  useEffect(() => {
    if (disabled[active] && selectable.length > 0) {
      onChange(selectable[0]);
    }
  }, [active, disabled, onChange, selectable]);

  return (
    <div
      role="tablist"
      aria-label={t("meeting.detail.tabs.label")}
      // Never truncate: on a narrow window the strip scrolls (§6.20).
      className="flex gap-1 overflow-x-auto border-b border-mid-gray/20"
      onKeyDown={(event) => {
        switch (event.key) {
          case "ArrowRight":
            event.preventDefault();
            go(stepTab(active, 1, selectable));
            break;
          case "ArrowLeft":
            event.preventDefault();
            go(stepTab(active, -1, selectable));
            break;
          case "Home":
            event.preventDefault();
            go(edgeTab("first", selectable));
            break;
          case "End":
            event.preventDefault();
            go(edgeTab("last", selectable));
            break;
          default:
            break;
        }
      }}
    >
      {MEETING_DETAIL_TABS.map((tab) => {
        const isDisabled = Boolean(disabled[tab]);
        const isActive = tab === active;
        return (
          <button
            key={tab}
            ref={(node) => {
              refs.current.set(tab, node);
            }}
            role="tab"
            type="button"
            id={`meeting-tab-${tab}`}
            aria-controls={`meeting-panel-${tab}`}
            aria-selected={isActive}
            aria-disabled={isDisabled || undefined}
            title={disabled[tab]}
            // Roving tabindex: one stop for the whole strip.
            tabIndex={isActive ? 0 : -1}
            disabled={isDisabled}
            onClick={() => !isDisabled && onChange(tab)}
            className={[
              // 44 px minimum target (§6.14).
              "min-h-11 px-4 text-sm whitespace-nowrap border-b-2 -mb-px",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary",
              isActive
                ? "border-logo-primary font-medium text-text"
                : "border-transparent text-text/60 hover:text-text",
              isDisabled ? "cursor-not-allowed opacity-40" : "",
            ].join(" ")}
          >
            {t(tabLabelKey(tab))}
          </button>
        );
      })}
    </div>
  );
};

interface MeetingDetailPanelProps {
  tab: MeetingDetailTab;
  active: MeetingDetailTab;
  children: React.ReactNode;
}

/** Panel wrapper that keeps the ARIA wiring next to the tablist above. */
export const MeetingDetailPanel: React.FC<MeetingDetailPanelProps> = ({
  tab,
  active,
  children,
}) => {
  if (tab !== active) return null;
  return (
    <div
      role="tabpanel"
      id={`meeting-panel-${tab}`}
      aria-labelledby={`meeting-tab-${tab}`}
      tabIndex={0}
      className="pt-4 focus-visible:outline-none"
    >
      {children}
    </div>
  );
};
