import { useEffect, useRef, useState } from "react";

// How long the selector's announcement text must stay unchanged before it
// counts as a *persistent* status worth announcing. Rapid flapping (e.g. a
// quick failed model switch) resets this timer instead of spamming the live
// region.
export const STATUS_SETTLE_DELAY_MS = 1500;

/**
 * Restraint-first narration text for the selector's polite live region.
 *
 * One message per status transition, with volatile numerals excluded so
 * per-tick download percentages can never reach assistive tech: while a
 * download runs, the text is a constant ("Downloading..." / "Downloading N
 * models...") that only changes when a download starts or ends. Every other
 * status already has a churn-free display text, which is reused verbatim.
 */
export const getStatusAnnouncementText = (args: {
  displayStatus: string;
  downloadCount: number;
  displayText: string;
  t: (key: string, options?: Record<string, unknown>) => string;
}): string => {
  const { displayStatus, downloadCount, displayText, t } = args;
  if (displayStatus !== "downloading") return displayText;
  return downloadCount > 1
    ? t("modelSelector.downloadingMultiple", { count: downloadCount })
    : t("modelSelector.downloadingGeneric");
};

interface StatusAnnouncement {
  /** Current message carried by the polite live region ("" before the first transition). */
  message: string;
  /**
   * Bumped on every announced transition. Used as the React key of the live
   * region's child so even a return to a previously-announced status replaces
   * the DOM node and is re-announced rather than swallowed as "no change".
   */
  nonce: number;
}

/**
 * Feeds the selector's polite live region from its announcement-text
 * transitions.
 *
 * Restraint rules baked in here:
 * - The state the app happens to open in is never announced (no narration at
 *   startup); only transitions after mount are.
 * - Text must stay stable for STATUS_SETTLE_DELAY_MS before it is announced,
 *   so transient flapping never reaches assistive tech.
 * - Each distinct settled text is announced once per transition.
 */
export const useModelStatusAnnouncer = (
  announcementText: string,
): StatusAnnouncement => {
  const [announcement, setAnnouncement] = useState<StatusAnnouncement>({
    message: "",
    nonce: 0,
  });
  // Last settled (announced-or-recorded) announcement text.
  const settledTextRef = useRef<string | null>(null);
  // False until the first settle window has elapsed: the initial status is
  // recorded silently instead of narrated.
  const pastInitialSettleRef = useRef(false);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      if (!pastInitialSettleRef.current) {
        pastInitialSettleRef.current = true;
        settledTextRef.current = announcementText;
        return;
      }
      if (settledTextRef.current === announcementText) return;
      settledTextRef.current = announcementText;
      setAnnouncement((prev) => ({
        message: announcementText,
        nonce: prev.nonce + 1,
      }));
    }, STATUS_SETTLE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [announcementText]);

  return announcement;
};
