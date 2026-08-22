import React, { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

type TooltipPosition = "top" | "bottom";

interface TooltipCoords {
  top: number;
  left: number;
  arrowLeft: number;
  actualPosition: TooltipPosition;
}

interface TooltipProps {
  targetRef: React.RefObject<HTMLElement>;
  position?: TooltipPosition;
  /**
   * Stable DOM id applied to the tooltip node. Triggers should reference it
   * via `aria-describedby` so assistive tech announces the description
   * (a11y D8). When omitted the tooltip renders without an id.
   */
  id?: string;
  children: React.ReactNode;
}

const TOOLTIP_WIDTH = 200;
const VIEWPORT_PADDING = 12;
const GAP = 8;
const ARROW_MARGIN = 12;
const DEFAULT_HEIGHT = 60;

/**
 * Accessible tooltip primitive (a11y D8).
 *
 * Owns all open/close semantics for its trigger element (`targetRef`):
 * - opens on pointer hover **and** on keyboard focus,
 * - closes on blur / pointer leave,
 * - a click on the trigger pins it until dismissal,
 * - Escape dismisses it without propagating to surrounding dialogs,
 * - a click outside trigger and tooltip dismisses it.
 *
 * The rendered node carries `role="tooltip"` and the given `id`, so call
 * sites can wire `aria-describedby` on the trigger ahead of time; the id
 * simply resolves once the tooltip is shown.
 */
export const Tooltip: React.FC<TooltipProps> = ({
  targetRef,
  position = "top",
  id,
  children,
}) => {
  const [coords, setCoords] = useState<TooltipCoords | null>(null);
  const [open, setOpen] = useState(false);
  const tooltipRef = useRef<HTMLDivElement>(null);
  // A click on the trigger pins the tooltip: hover/blur no longer hide it.
  const pinnedRef = useRef(false);

  // Hover + focus + click semantics, attached directly to the trigger
  // element. focusin/focusout are used because they bubble from the
  // actual focusable control inside the trigger wrapper.
  useEffect(() => {
    const target = targetRef.current;
    if (!target) return;

    const show = () => setOpen(true);
    const hide = () => {
      if (pinnedRef.current) return;
      setOpen(false);
    };
    const handleClick = () => {
      if (pinnedRef.current) {
        pinnedRef.current = false;
        setOpen(false);
      } else {
        pinnedRef.current = true;
        setOpen(true);
      }
    };
    // Blur (focusout) always dismisses, even a pinned tooltip: keyboard
    // users must never leave a stray tooltip behind.
    const handleFocusOut = () => {
      pinnedRef.current = false;
      setOpen(false);
    };

    target.addEventListener("mouseenter", show);
    target.addEventListener("mouseleave", hide);
    target.addEventListener("focusin", show);
    target.addEventListener("focusout", handleFocusOut);
    target.addEventListener("click", handleClick);

    return () => {
      target.removeEventListener("mouseenter", show);
      target.removeEventListener("mouseleave", hide);
      target.removeEventListener("focusin", show);
      target.removeEventListener("focusout", handleFocusOut);
      target.removeEventListener("click", handleClick);
    };
  }, [targetRef]);

  // While open: Escape dismisses without closing surrounding dialogs
  // (capture phase stops propagation before dialog handlers see it),
  // and a click outside trigger + tooltip dismisses it.
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      pinnedRef.current = false;
      setOpen(false);
    };
    const handleMouseDown = (event: MouseEvent) => {
      const node = event.target as Node;
      if (tooltipRef.current?.contains(node)) return;
      if (targetRef.current?.contains(node)) return;
      pinnedRef.current = false;
      setOpen(false);
    };

    document.addEventListener("keydown", handleKeyDown, true);
    document.addEventListener("mousedown", handleMouseDown);

    return () => {
      document.removeEventListener("keydown", handleKeyDown, true);
      document.removeEventListener("mousedown", handleMouseDown);
    };
  }, [open, targetRef]);

  const updatePosition = useCallback(() => {
    if (!targetRef.current) return;

    const targetRect = targetRef.current.getBoundingClientRect();
    const tooltipHeight = tooltipRef.current?.offsetHeight || DEFAULT_HEIGHT;

    let actualPosition = position;
    let top: number;

    if (position === "top") {
      const spaceAbove = targetRect.top - tooltipHeight - GAP;
      if (spaceAbove < VIEWPORT_PADDING) {
        actualPosition = "bottom";
        top = targetRect.bottom + GAP;
      } else {
        top = targetRect.top - GAP - tooltipHeight;
      }
    } else {
      const spaceBelow =
        window.innerHeight - targetRect.bottom - tooltipHeight - GAP;
      if (spaceBelow < VIEWPORT_PADDING) {
        actualPosition = "top";
        top = targetRect.top - GAP - tooltipHeight;
      } else {
        top = targetRect.bottom + GAP;
      }
    }

    const targetCenter = targetRect.left + targetRect.width / 2;
    let left = targetCenter - TOOLTIP_WIDTH / 2;

    if (left < VIEWPORT_PADDING) {
      left = VIEWPORT_PADDING;
    } else if (left + TOOLTIP_WIDTH > window.innerWidth - VIEWPORT_PADDING) {
      left = window.innerWidth - TOOLTIP_WIDTH - VIEWPORT_PADDING;
    }

    const arrowLeft = Math.min(
      Math.max(targetCenter - left, ARROW_MARGIN),
      TOOLTIP_WIDTH - ARROW_MARGIN,
    );

    setCoords({ top, left, arrowLeft, actualPosition });
  }, [targetRef, position]);

  useEffect(() => {
    if (!open) return;

    updatePosition();

    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("resize", updatePosition);

    return () => {
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("resize", updatePosition);
    };
  }, [open, updatePosition]);

  if (!open) return null;

  const arrowClasses =
    coords?.actualPosition === "top" ? "top-full" : "bottom-full rotate-180";

  return createPortal(
    <div
      id={id}
      role="tooltip"
      ref={tooltipRef}
      style={{
        position: "fixed",
        top: coords?.top ?? -9999,
        left: coords?.left ?? -9999,
        width: TOOLTIP_WIDTH,
        zIndex: 9999,
        opacity: coords ? 1 : 0,
      }}
      className="px-3 py-2 bg-background border border-mid-gray/80 rounded-lg shadow-lg whitespace-normal transition-opacity duration-150"
    >
      {children}
      <div
        style={{ left: coords?.arrowLeft ?? 0 }}
        className={`absolute ${arrowClasses} transform -translate-x-1/2 w-0 h-0 border-l-[6px] border-r-[6px] border-t-[6px] border-l-transparent border-r-transparent border-t-mid-gray/80`}
      />
    </div>,
    document.body,
  );
};
