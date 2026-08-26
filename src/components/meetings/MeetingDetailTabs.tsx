import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/** The four detail tabs of plan §6.13, in their fixed order. */
export const MEETING_DETAIL_TABS = [
  "overview",
  "transcript",
  "audio",
  "processing",
] as const;

export type MeetingDetailTab = (typeof MEETING_DETAIL_TABS)[number];

/** i18n key of a tab's visible label. */
export const tabLabelKey = (tab: MeetingDetailTab): string =>
  `meeting.detail.tabs.${tab}`;

/** Tabs a user can actually reach, in display order. */
export const selectableTabs = (
  disabled: Partial<Record<MeetingDetailTab, string>>,
): MeetingDetailTab[] => MEETING_DETAIL_TABS.filter((tab) => !disabled[tab]);

/**
 * Roving-tabindex step: the tab `delta` positions away from `active`, wrapping
 * around and skipping unreachable tabs.
 *
 * Pure so the keyboard contract (§6.14 "vollständige Tastaturbedienung") is
 * testable without a DOM. An `active` tab that is itself unreachable — the
 * caller is about to correct that — steps relative to the first reachable one.
 */
export const stepTab = (
  active: MeetingDetailTab,
  delta: number,
  selectable: MeetingDetailTab[],
): MeetingDetailTab | null => {
  if (selectable.length === 0) return null;
  const current = selectable.indexOf(active);
  const base = current === -1 ? 0 : current;
  const index = (base + delta) % selectable.length;
  return selectable[index < 0 ? index + selectable.length : index];
};

/** Home/End target among the reachable tabs. */
export const edgeTab = (
  edge: "first" | "last",
  selectable: MeetingDetailTab[],
): MeetingDetailTab | null => {
  if (selectable.length === 0) return null;
  return edge === "first" ? selectable[0] : selectable[selectable.length - 1];
};

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

/** Local state hook so the detail view stays declarative. */
export const useMeetingDetailTab = (resetKey: string) => {
  const [tab, setTab] = useState<MeetingDetailTab>("overview");
  const lastKey = useRef(resetKey);
  useEffect(() => {
    if (lastKey.current !== resetKey) {
      lastKey.current = resetKey;
      // Opening a different meeting starts on the plain-language tab again.
      setTab("overview");
    }
  }, [resetKey]);
  return [tab, setTab] as const;
};
