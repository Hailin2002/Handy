import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RecordingOverlay.css";
import { commands, events } from "@/bindings";
import type {
  AssistantAnswerEvent,
  StreamPhase,
  StreamPhaseEvent,
  StreamTextEvent,
  StreamWorkKind,
} from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";

type OverlayState =
  | "recording"
  | "streaming"
  | "transcribing"
  | "processing"
  // Meetings get their own states rather than reusing "recording": the two
  // tools must never be confusable (plan §6.10), and the meeting form shows a
  // duration where dictation shows a waveform.
  | "meeting"
  | "meeting_paused"
  // The assistant result card. Unlike every other state this one is read rather
  // than glanced at, and it stays up until the user dismisses it — see
  // ANSWER_CARD_OPEN in overlay.rs.
  | "answer";

// Number of reactive bars in the waveform (the simple, smoothed style shared by
// every overlay form). Mic levels arrive as 16 FFT buckets; we take the first N.
const WAVE_BARS = 9;

// Profile id of the smart assistant, mirroring ASSISTANT_PROFILE_SMART in
// src-tauri/src/settings/types.rs. The card hides its "ask the smart assistant"
// button when the answer already came from there — offering to escalate an
// answer to the model that just produced it is a dead end.
const SMART_PROFILE_ID = "smart_assistant";

// How long a Copy / Paste / Replace confirmation stays on the card.
const NOTICE_MS = 2400;

/** The shape every generated `commands.*` call resolves to. */
type CommandResult<T> =
  { status: "ok"; data: T } | { status: "error"; error: string };

/**
 * The assistant result card.
 *
 * Exported and kept separate from the overlay shell for two reasons: it is the
 * only overlay form with enough state of its own to be worth isolating, and it
 * is the only one that can be asserted in a test — the shell renders `null`
 * until an event listener fires, which never happens under `react-dom/server`.
 *
 * Every action goes through a Tauri command rather than a browser API. Only the
 * Windows answer state becomes focusable, and the native command path keeps
 * clipboard behavior consistent while restoring the original target before a
 * paste. The visible Close button stays as the cross-platform exit. See
 * `commands/assistant.rs`.
 */
export const AssistantAnswerCard: React.FC<{
  answer: AssistantAnswerEvent;
}> = ({ answer }) => {
  const { t } = useTranslation();
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [escalating, setEscalating] = useState(false);
  const [linksOpen, setLinksOpen] = useState(false);

  // A fresh event — an escalation landing — replaces the card contents, so every
  // piece of per-answer UI state has to start over with it. Without this, an
  // escalated answer would arrive under the previous answer's "Copied"
  // confirmation and a still-pending escalate button.
  useEffect(() => {
    setNotice(null);
    setBusy(false);
    setEscalating(false);
    setLinksOpen(false);
  }, [answer]);

  useEffect(() => {
    if (!notice) return;
    const id = setTimeout(() => setNotice(null), NOTICE_MS);
    return () => clearTimeout(id);
  }, [notice]);

  const text = answer.answer ?? "";
  const links = answer.links;
  const isSmart = answer.profile_id === SMART_PROFILE_ID;

  // Confirmations are the point: a clipboard or synthetic-key action can have
  // no visible effect inside the card, so a click that produced no feedback is
  // indistinguishable from a click that missed.
  const runAction = async (
    invoke: () => Promise<CommandResult<null>>,
    confirmation: string,
  ) => {
    setBusy(true);
    try {
      const result = await invoke();
      setNotice(result.status === "ok" ? confirmation : result.error);
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  };

  const openLink = async (url: string) => {
    try {
      await openUrl(url);
    } catch (error) {
      setNotice(String(error));
    }
  };

  const escalate = async () => {
    setEscalating(true);
    setNotice(null);
    try {
      const result = await commands.assistantEscalate(answer.question);
      if (result.status === "error") {
        setNotice(result.error);
        setEscalating(false);
      }
      // On success the answer arrives as a fresh event and resets this
      // component through the effect above. A `false` payload means the click
      // was coalesced into a request already in flight, which does the same.
    } catch (error) {
      setNotice(String(error));
      setEscalating(false);
    }
  };

  const handleCardKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (text.length === 0 || !(event.ctrlKey || event.metaKey)) return;

    const key = event.key.toLowerCase();
    if (key === "c") {
      event.preventDefault();
      void runAction(
        () => commands.assistantCopy(text),
        t("overlay.assistant.copied"),
      );
    } else if (key === "v") {
      event.preventDefault();
      void runAction(
        () => commands.assistantPaste(text),
        t("overlay.assistant.pasted"),
      );
    }
  };

  return (
    <div
      className="scard answer"
      tabIndex={0}
      aria-keyshortcuts="Control+C Meta+C Control+V Meta+V"
      onKeyDown={handleCardKeyDown}
    >
      <div className="oa">
        <div className="oa-head">
          <span className="oa-kicker">
            {isSmart
              ? t("overlay.assistant.smartLabel")
              : t("overlay.assistant.label")}
          </span>
          {/* The question is shown because the most common cause of a
              surprising answer is a misheard question. */}
          <span className="oa-q" title={answer.question}>
            {answer.question}
          </span>
        </div>

        {answer.error ? (
          <div className="oa-body error" role="alert">
            {answer.error}
          </div>
        ) : (
          <div className="oa-body" role="status" aria-live="polite">
            {text}
          </div>
        )}

        {answer.truncated && (
          <div className="oa-truncated" role="note">
            {t("overlay.assistant.truncated")}
          </div>
        )}

        {linksOpen && links.length > 1 && (
          <div className="oa-links">
            {links.map((url) => (
              <button
                key={url}
                type="button"
                className="oa-link"
                title={url}
                onClick={() => openLink(url)}
              >
                {url}
              </button>
            ))}
          </div>
        )}

        <div className="oa-note" role="status" aria-live="polite">
          {escalating ? t("overlay.assistant.asking") : notice}
        </div>

        <div className="oa-actions">
          {text.length > 0 && (
            <>
              <button
                type="button"
                className="oa-btn"
                disabled={busy}
                onClick={() =>
                  runAction(
                    () => commands.assistantCopy(text),
                    t("overlay.assistant.copied"),
                  )
                }
              >
                {t("overlay.assistant.copy")}
              </button>
              <button
                type="button"
                className="oa-btn"
                disabled={busy}
                onClick={() =>
                  runAction(
                    () => commands.assistantPaste(text),
                    t("overlay.assistant.pasted"),
                  )
                }
              >
                {t("overlay.assistant.paste")}
              </button>
              {/* Replace overwrites the selection in whatever window happens to
                  be focused, so it is its own explicitly labelled button and
                  never a default. */}
              <button
                type="button"
                className="oa-btn"
                disabled={busy}
                title={t("overlay.assistant.replaceHint")}
                onClick={() =>
                  runAction(
                    () => commands.assistantReplace(text),
                    t("overlay.assistant.replaced"),
                  )
                }
              >
                {t("overlay.assistant.replace")}
              </button>
            </>
          )}

          {links.length === 1 && (
            <button
              type="button"
              className="oa-btn"
              onClick={() => openLink(links[0])}
            >
              {t("overlay.assistant.openLink")}
            </button>
          )}
          {links.length > 1 && (
            <button
              type="button"
              className="oa-btn"
              aria-expanded={linksOpen}
              onClick={() => setLinksOpen((shown) => !shown)}
            >
              {t("overlay.assistant.chooseLink", { n: links.length })}
            </button>
          )}

          {!isSmart && (
            <button
              type="button"
              className="oa-btn accent"
              disabled={busy || escalating}
              onClick={escalate}
            >
              {t("overlay.assistant.escalate")}
            </button>
          )}

          <button
            type="button"
            className="oa-btn close"
            onClick={() => commands.assistantDismiss()}
          >
            {t("overlay.assistant.close")}
          </button>
        </div>
      </div>
    </div>
  );
};

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  // `Stream::play()` returning does not mean hardware callbacks are flowing.
  // Stay visually in an arming state until the backend processes the first
  // actual microphone sample chunk.
  const [captureReady, setCaptureReady] = useState(false);
  const [levels, setLevels] = useState<number[]>(Array(WAVE_BARS).fill(0));
  const [streamText, setStreamText] = useState<StreamTextEvent>({
    committed: "",
    tentative: "",
  });
  const [phase, setPhase] = useState<StreamPhase>("listening");
  const [workKind, setWorkKind] = useState<StreamWorkKind>("transcribing");
  const [elapsed, setElapsed] = useState(0);
  // Bumped on each new streaming session so the Live card remounts fresh (replays
  // the pop-in, and never animates in from the previous panel's open size).
  const [session, setSession] = useState(0);
  // Overlay placement (top vs bottom of the screen). The Live panel grows downward
  // from a top overlay (oldest line under the pill) and upward from a bottom one.
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  // True once live text overflows the cap. A top overlay fades its top edge only
  // while overflowing, so the resting first line stays crisp flush under the pill.
  const [overflowing, setOverflowing] = useState(false);
  // The assistant answer currently on the card, if any.
  const [answer, setAnswer] = useState<AssistantAnswerEvent | null>(null);
  // Mirrors "a card is on screen" for the answer listener, which is registered
  // once on mount and would otherwise close over the initial state forever.
  const cardOpenRef = useRef(false);

  const smoothedLevelsRef = useRef<number[]>(Array(16).fill(0));
  // Live-text scroll-back: the text region "sticks" to the newest line while the
  // user is at the bottom; if they scroll up to read history, auto-follow pauses
  // until they scroll back down.
  const capRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    const setupEventListeners = async () => {
      const unlistenShow = await listen("show-overlay", async (event) => {
        const overlayState = event.payload as OverlayState;
        // Reset synchronously before settings I/O. A fast microphone can emit
        // recording-ready while the awaits below are in flight; resetting after
        // them would overwrite that event and leave the overlay stuck arming.
        if (overlayState === "recording" || overlayState === "streaming") {
          setCaptureReady(false);
          smoothedLevelsRef.current = Array(16).fill(0);
          setLevels(Array(WAVE_BARS).fill(0));
          setStreamText({ committed: "", tentative: "" });
        }
        // Any other state supersedes an open card (overlay.rs releases the
        // card's hold on the window at the same moment), so drop the payload
        // rather than leaving a stale answer to reappear later.
        if (overlayState !== "answer") {
          setAnswer(null);
        }

        await syncLanguageFromSettings();
        // The Live panel flows downward from a top overlay and upward from a
        // bottom one; read the placement so the layout can flip to match.
        try {
          const settings = await commands.getAppSettings();
          if (settings.status === "ok") {
            setPosition(
              settings.data.overlay_position === "top" ? "top" : "bottom",
            );
          }
        } catch {
          // Keep the previous/default placement if settings can't be read.
        }
        setState(overlayState);
        if (overlayState === "streaming") {
          setPhase("listening");
          setWorkKind("transcribing");
          setElapsed(0);
          setSession((s) => s + 1); // remount the card fresh for this session
        }
        setIsVisible(true);
      });

      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
        setCaptureReady(false);
        setAnswer(null);
      });

      const unlistenReady = await listen("recording-ready", () => {
        setElapsed(0);
        setCaptureReady(true);
      });

      const unlistenLevel = await listen<number[]>("mic-level", (event) => {
        const newLevels = event.payload as number[];
        // Exponential smoothing across the 16 buckets, then take the first N
        // bars for the shared waveform.
        const smoothed = smoothedLevelsRef.current.map((prev, i) => {
          const target = newLevels[i] || 0;
          return prev * 0.7 + target * 0.3;
        });
        smoothedLevelsRef.current = smoothed;
        setLevels(smoothed.slice(0, WAVE_BARS));
      });

      const unlistenStream = await events.streamTextEvent.listen((event) => {
        setStreamText(event.payload);
      });

      const unlistenPhase = await events.streamPhaseEvent.listen((event) => {
        const payload: StreamPhaseEvent = event.payload;
        setPhase(payload.phase);
        if (payload.kind) setWorkKind(payload.kind);
      });

      // Every assistant run emits this, including the Copy and Paste modes that
      // deliver without a card and hide the overlay themselves. Whether a card
      // is warranted is therefore the backend's call, not a rule duplicated
      // here: `assistantPresentAnswer` resizes and shows the overlay and reports
      // back whether it did.
      const unlistenAnswer = await events.assistantAnswerEvent.listen(
        async (event) => {
          const payload = event.payload;
          if (cardOpenRef.current) {
            // An escalation refreshing the card the user is already reading.
            // The window is already the right size and already visible.
            setAnswer(payload);
            return;
          }
          // Deliberately set only once the backend confirms a card. The
          // assistant failure paths emit this event and then hide the overlay,
          // so a `hide-overlay` is usually already on its way and would clear an
          // answer stored before it. Presenting first orders the two correctly:
          // `show-overlay` re-raises the window, and the payload lands after.
          // The emitter decided whether a card is warranted and said so in
          // the payload. A Copy/Paste run, or one with the overlay switched
          // off, is delivered elsewhere and hides the overlay itself.
          if (!payload.expects_card) return;
          try {
            const shown = await commands.assistantPresentAnswer();
            if (shown) setAnswer(payload);
          } catch {
            // Nothing was stored, so there is nothing to roll back.
          }
        },
      );

      return () => {
        unlistenShow();
        unlistenHide();
        unlistenReady();
        unlistenLevel();
        unlistenStream();
        unlistenPhase();
        unlistenAnswer();
      };
    };

    setupEventListeners();
  }, []);

  // Elapsed capture timer starts only once microphone samples are flowing.
  useEffect(() => {
    if (state !== "streaming" || !isVisible || !captureReady) return;
    const id = setInterval(() => setElapsed((e) => e + 1), 1000);
    return () => clearInterval(id);
  }, [state, isVisible, captureReady]);

  // Stick to the bottom as text streams in — but only while pinned, so a user who
  // has scrolled up to read history isn't yanked back down by the next chunk.
  useLayoutEffect(() => {
    const el = capRef.current;
    if (!el) return;
    // Fade the top edge only once text actually overflows the cap.
    setOverflowing(el.scrollHeight > el.clientHeight + 1);
    if (pinnedRef.current) el.scrollTop = el.scrollHeight;
  }, [streamText]);

  // Each fresh streaming session starts pinned to the bottom, fade cleared.
  useEffect(() => {
    pinnedRef.current = true;
    setOverflowing(false);
  }, [session]);

  useEffect(() => {
    cardOpenRef.current = isVisible && state === "answer";
  }, [isVisible, state]);

  if (!isVisible) return null;

  // Re-pin when the user is within ~a line of the bottom; unpin otherwise.
  const handleStreamScroll = () => {
    const el = capRef.current;
    if (!el) return;
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 16;
  };

  const fmtTime = (s: number) =>
    `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;

  // ---- Shared building blocks (one visual language for every overlay form) ----
  const waveform = (
    <div className={`swave ${captureReady ? "ready" : "arming"}`}>
      {levels.map((v, i) => (
        <i
          key={i}
          style={{
            height: `${Math.max(3, Math.min(18, 3 + Math.pow(v, 0.7) * 15))}px`,
          }}
        />
      ))}
    </div>
  );

  const cancelBtn = (
    <button
      className="sx"
      aria-label={t("overlay.cancel")}
      onClick={() => commands.cancelOperation()}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path
          d="M4 4 L12 12 M12 4 L4 12"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    </button>
  );

  // dot (left) | waveform (center) | timer + cancel (right) — same structure for
  // pill & panel, so the Live morph is a pure width change.
  const listeningRow = (showTimer: boolean, showCancel: boolean) => (
    <div className="sbase">
      <div className="sbase-l">
        <span className={`sdot ${captureReady ? "ready" : "arming"}`} />
      </div>
      {waveform}
      <div className="sbase-r">
        {showTimer && <span className="stimer">{fmtTime(elapsed)}</span>}
        {showCancel && cancelBtn}
      </div>
    </div>
  );

  // dot (left) | label (center) | timer (right) — the meeting form.
  //
  // Deliberately different from the dictation rows: no waveform, no cancel
  // button. A meeting is stopped from the tray or the window, never from a
  // non-focusable overlay where a stray click could end a two-hour recording
  // (plan §6.9). The label plus the dot carry the state, so paused and
  // recording never differ by colour alone (§6.8).
  const meetingRow = (paused: boolean) => (
    <div className="sbase">
      <div className="sbase-l">
        <span className={`sdot ${paused ? "arming" : "ready"}`} />
      </div>
      <span className="swork-label" role="status">
        {paused ? t("overlay.meetingPaused") : t("overlay.meetingRecording")}
      </span>
      <div className="sbase-r">
        <span className="stimer">{fmtTime(elapsed)}</span>
      </div>
    </div>
  );

  // spinner (left) | label (center) | cancel (right) — same 3-zone grid as the
  // listening row, so the label is centered.
  const workingRow = (label: string, showCancel: boolean) => (
    <div className="sbase">
      <div className="sbase-l">
        <span className="sspinner" />
      </div>
      <span className="swork-label" role="status">
        {label}
      </span>
      <div className="sbase-r">{showCancel && cancelBtn}</div>
    </div>
  );

  // ---- Assistant answer card: prose plus actions, no waveform, no timer ----
  if (state === "answer") {
    // The show-overlay event can land a beat before the payload does, and a
    // stray hide-overlay can clear the payload while the native window is
    // still up. Never render nothing here: an empty, transparent,
    // always-on-top window eats clicks and offers no way to close it.
    if (!answer) {
      return (
        <div dir={direction} className={`ov-stage ${position} ov-fade show`}>
          <div className="scard answer">
            <div className="oa oa-empty">
              <button
                type="button"
                className="oa-btn close"
                onClick={() => commands.assistantDismiss()}
              >
                {t("overlay.assistant.close")}
              </button>
            </div>
          </div>
        </div>
      );
    }
    return (
      <div dir={direction} className={`ov-stage ${position} ov-fade show`}>
        <AssistantAnswerCard answer={answer} />
      </div>
    );
  }

  // ---- Live overlay: a pill that sculpts open into a panel ----
  if (state === "streaming") {
    const hasText =
      streamText.committed.length > 0 || streamText.tentative.length > 0;
    const working = phase === "working";
    // Keep the panel open whenever there's text — even while finalizing — so the
    // transcript stays put under a working spinner instead of collapsing and
    // squishing the text mid-stream. Only fall back to the small working pill
    // when there was no text to preserve.
    const open = hasText;
    const collapsed = working && !hasText;

    return (
      <div dir={direction} className={`ov-stage ${position}`}>
        <div
          key={session}
          className={`scard ${open ? "open" : ""} ${collapsed ? "working" : ""} ${
            isVisible ? "" : "leaving"
          }`}
        >
          <div className="stext">
            <div className="stext-clip">
              <div
                className={`stext-cap ${overflowing ? "overflowing" : ""}`}
                ref={capRef}
                onScroll={handleStreamScroll}
                aria-live="polite"
              >
                <p>
                  <span className="committed">
                    {streamText.committed ? streamText.committed + " " : ""}
                  </span>
                  <span className="tentative">{streamText.tentative}</span>
                  {/* Drop the blinking caret once finalizing — it's no longer
                      capturing, and a static spinner conveys the work. */}
                  {!working && <span className="scaret" />}
                </p>
              </div>
            </div>
          </div>
          {working
            ? workingRow(
                workKind === "polishing"
                  ? t("overlay.processing")
                  : t("overlay.transcribing"),
                true,
              )
            : listeningRow(open, true)}
        </div>
      </div>
    );
  }

  // ---- Minimal overlay: exactly one row at a time — waveform (recording), or a
  // spinner + label (transcribing / processing). Never both. The pill animates its
  // width between them; the cancel button is in both rows so it stays put.
  const working = state === "transcribing" || state === "processing";
  const meeting = state === "meeting" || state === "meeting_paused";
  const workLabel =
    state === "processing"
      ? t("overlay.processing")
      : t("overlay.transcribing");

  const row = meeting
    ? meetingRow(state === "meeting_paused")
    : working
      ? workingRow(workLabel, true)
      : listeningRow(false, true);

  return (
    <div
      dir={direction}
      className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
    >
      <div
        className={`scard compact ${(working || meeting) && isVisible ? "cworking" : ""}`}
      >
        {row}
      </div>
    </div>
  );
};

export default RecordingOverlay;
