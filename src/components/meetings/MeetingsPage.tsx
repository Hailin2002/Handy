import React, { useCallback, useEffect, useRef, useState } from "react";
import { WhatsAppImportDialog } from "@/components/sources/WhatsAppImportDialog";
import { Plus, Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type MeetingSession } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { formatDateTime } from "@/utils/dateFormat";
import { isMeetingCapturing, useMeetingStore } from "@/stores/meetingStore";
import { meetingStateLabelKey } from "./meetingStates";
import { MeetingDetail } from "./MeetingDetail";
import { MeetingList } from "./MeetingList";
import { MeetingProcessingView } from "./MeetingProcessingView";
import { MeetingRecorderView } from "./MeetingRecorderView";
import { MeetingRecoveryBanner } from "./MeetingRecoveryBanner";
import { MeetingSetup } from "./MeetingSetup";

/**
 * Meetings page and session router (plan §6.5–§6.12).
 *
 * The library itself (list, search, create, open, rename, delete) is the
 * wave-1 CRUD journey. On top of it this component routes the *session*
 * flow, deriving the destination from `meetingStore` rather than from local
 * navigation state — because the plan makes the backend state machine the
 * single authority for what the user sees (line 926 f.):
 *
 *   draft                              → MeetingSetup (compact start dialog)
 *   starting/recording/paused/stopping → MeetingRecorderView
 *   transcribing/summarizing/completed → MeetingProcessingView
 *   otherwise                          → library or detail view
 *
 * Interrupted sessions surface above the library through
 * `MeetingRecoveryBanner`, so a crashed recording is offered for processing
 * instead of silently disappearing.
 */
export const MeetingsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [sessions, setSessions] = useState<MeetingSession[] | null>(null);
  const [query, setQuery] = useState("");
  // True once the user has run a search, until the next unfiltered refresh.
  // Keeps the list chrome (search box, "no results" feedback) stable while a
  // filter is active — clearing the input alone must not swap the view back
  // to the first-run empty library before the search button is even clicked.
  const [isFiltered, setIsFiltered] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [setupSession, setSetupSession] = useState<MeetingSession | null>(null);
  const [creating, setCreating] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const createInFlight = useRef(false);

  const liveSession = useMeetingStore((state) => state.session);
  const initListeners = useMeetingStore((state) => state.initListeners);
  const refreshRecoveries = useMeetingStore((state) => state.refreshRecoveries);
  const releaseSession = useMeetingStore((state) => state.releaseSession);

  const refresh = useCallback(
    async (q?: string) => {
      setIsFiltered(Boolean(q));
      try {
        const result = await commands.meetingListSessions(q || null);
        if (result.status === "ok") {
          setSessions(result.data);
        } else {
          setSessions([]);
          toast.error(t("meetings.loadError"), { description: result.error });
        }
      } catch (error) {
        console.error("Failed to load meetings:", error);
        setSessions([]);
        toast.error(t("meetings.loadError"));
      }
    },
    [t],
  );

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Subscribe-then-pull handshake (FSC L6): listeners are registered before
  // the recovery offers are fetched, so an offer emitted in between is not
  // lost. Listener disposal is symmetric and owned by the store.
  useEffect(() => {
    void initListeners().then(() => refreshRecoveries());
  }, [initListeners, refreshRecoveries]);

  const handleSearch = async () => {
    await refresh(query.trim() || undefined);
  };

  const createMeeting = async () => {
    // Synchronous re-entry guard: `creating` state alone cannot stop a
    // same-frame double-click from creating two draft rows.
    if (creating || createInFlight.current) return;
    createInFlight.current = true;
    setCreating(true);
    try {
      // Plan §6.7 line 508: title pre-filled with date and time; formatted
      // through Intl so it follows the user's locale without new i18n keys.
      const defaultTitle = new Intl.DateTimeFormat(i18n.language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date());
      const result = await commands.meetingCreateSession(defaultTitle);
      if (result.status === "ok") {
        // §6.6: after creating, the user lands directly in the compact start
        // dialog rather than in a technical configuration screen.
        setSetupSession(result.data);
        await refresh();
      } else {
        toast.error(t("meetings.createError"), { description: result.error });
      }
    } catch (error) {
      console.error("Failed to create meeting:", error);
      toast.error(t("meetings.createError"));
    } finally {
      createInFlight.current = false;
      setCreating(false);
    }
  };

  // --- session flow (authority: meetingStore) ---------------------------
  if (liveSession && isMeetingCapturing(liveSession.state)) {
    return (
      <MeetingRecorderView
        onStopped={() => {
          setSetupSession(null);
          void refresh();
        }}
      />
    );
  }

  if (
    liveSession &&
    ["transcribing", "summarizing", "completed"].includes(liveSession.state)
  ) {
    return (
      <MeetingProcessingView
        onContinueInBackground={() => {
          releaseSession();
          void refresh();
        }}
        onOpenMeeting={(meetingId) => {
          releaseSession();
          setSelectedId(meetingId);
          void refresh();
        }}
      />
    );
  }

  if (setupSession) {
    return (
      <MeetingSetup
        session={setupSession}
        onBack={() => {
          setSetupSession(null);
          void refresh();
        }}
        onStarted={() => setSetupSession(null)}
        onSaved={(updated) => {
          setSetupSession(updated);
          void refresh();
        }}
      />
    );
  }

  const selected =
    selectedId && sessions
      ? sessions.find((s) => s.id === selectedId)
      : undefined;

  if (selectedId && !selected) {
    // Detail open but the list cache doesn't hold the session yet (fresh
    // creation before refresh resolves): fetch it directly.
    return (
      <MeetingDetailLoader
        id={selectedId}
        onBack={() => setSelectedId(null)}
        onSaved={(updated) => {
          void refresh();
          setSelectedId(updated.id);
        }}
        onDeleted={(deletedId) => {
          setSelectedId(null);
          void refresh();
          void deletedId;
        }}
        onLoaded={(loaded) => {
          setSessions((prev) =>
            prev
              ? [loaded, ...prev.filter((s) => s.id !== loaded.id)]
              : [loaded],
          );
        }}
      />
    );
  }

  if (selected) {
    return (
      <MeetingDetail
        session={selected}
        onBack={() => {
          setSelectedId(null);
          void refresh();
        }}
        onSaved={(updated) => {
          setSessions((prev) =>
            prev
              ? prev.map((s) => (s.id === updated.id ? updated : s))
              : [updated],
          );
        }}
        onDeleted={() => {
          setSelectedId(null);
          void refresh();
        }}
        onStartMeeting={(session) => setSetupSession(session)}
      />
    );
  }

  if (sessions === null) {
    return <p className="text-sm text-text/60">{t("common.loading")}</p>;
  }

  if (sessions.length === 0 && !isFiltered && !query.trim()) {
    // First-run empty library — only when no search filter is active, so a
    // no-match search shows the "no results" feedback below instead of
    // pretending every meeting vanished. The recovery banner stays visible
    // even here: an interrupted meeting must never be hidden behind the
    // first-run hero.
    return (
      <div className="flex w-full flex-col gap-4">
        <MeetingRecoveryBanner onOpenMeeting={(id) => setSelectedId(id)} />
        {/* Importing a chat export is a legitimate *first* action — arguably
            more so than recording — but the header carrying that button lives
            below this early return, so on an empty library the importer was
            unreachable. The hero's own secondary slot is reserved for the
            not-yet-shipped audio importer, so the button sits above it rather
            than duplicating the hero's primary "New Meeting" call to action. */}
        <div className="flex justify-end">
          <Button variant="secondary" onClick={() => setImportOpen(true)}>
            {t("whatsapp.title")}
          </Button>
        </div>
        <MeetingList onNewMeeting={createMeeting} />
        <WhatsAppImportDialog
          open={importOpen}
          onOpenChange={setImportOpen}
          onImported={() => void refresh()}
        />
      </div>
    );
  }

  return (
    <div className="flex w-full flex-col gap-4">
      <MeetingRecoveryBanner onOpenMeeting={(id) => setSelectedId(id)} />

      <header className="flex w-full items-center justify-between gap-4">
        <h2 className="text-lg font-semibold">{t("nav.meetings")}</h2>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={() => setImportOpen(true)}>
            {t("whatsapp.title")}
          </Button>
          <Button onClick={createMeeting} disabled={creating}>
            <Plus className="mr-1 h-4 w-4" aria-hidden="true" />
            {t("meetings.newMeeting")}
          </Button>
        </div>
      </header>

      <WhatsAppImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={() => void refresh()}
      />

      <div className="flex items-center gap-2">
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void handleSearch();
          }}
          placeholder={t("meetings.searchPlaceholder")}
          aria-label={t("meetings.searchPlaceholder")}
          className="max-w-xs"
        />
        <Button variant="secondary" size="sm" onClick={handleSearch}>
          <Search className="mr-1 h-4 w-4" aria-hidden="true" />
          {t("meetings.search")}
        </Button>
      </div>

      {sessions.length === 0 ? (
        <p className="text-sm text-text/60">{t("common.noOptionsFound")}</p>
      ) : (
        <ul
          className="flex w-full flex-col gap-2"
          aria-label={t("nav.meetings")}
        >
          {sessions.map((session) => (
            <li key={session.id}>
              <button
                type="button"
                onClick={() => setSelectedId(session.id)}
                className="w-full rounded-lg border border-mid-gray/20 bg-mid-gray/5 px-4 py-3 text-start transition-colors hover:border-logo-primary hover:bg-background-ui/30 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary cursor-pointer"
              >
                <span className="block truncate font-medium">
                  {session.title}
                </span>
                <span className="mt-0.5 flex items-center gap-3 text-xs text-text/60">
                  <span>
                    {formatDateTime(
                      String(Math.floor(session.created_at / 1000)),
                      i18n.language,
                    )}
                  </span>
                  <span>{t(meetingStateLabelKey(session.state))}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

/** Fetches one session by id when the list cache is not yet populated. */
const MeetingDetailLoader: React.FC<{
  id: string;
  onBack: () => void;
  onSaved: (session: MeetingSession) => void;
  onDeleted: (id: string) => void;
  onLoaded: (session: MeetingSession) => void;
}> = ({ id, onBack, onSaved, onDeleted, onLoaded }) => {
  const { t } = useTranslation();
  const [session, setSession] = useState<MeetingSession | null | undefined>(
    undefined,
  );

  // The fetch below must fire once per `id`, not whenever the parent re-renders
  // and hands down a fresh `onLoaded` identity. Keeping the callback in a ref
  // lets the effect depend on `id` alone while still calling the current one.
  const onLoadedRef = useRef(onLoaded);
  useEffect(() => {
    onLoadedRef.current = onLoaded;
  });

  useEffect(() => {
    let cancelled = false;
    commands
      .meetingGetSession(id)
      .then((result) => {
        if (cancelled) return;
        if (result.status === "ok" && result.data) {
          setSession(result.data);
          onLoadedRef.current(result.data);
        } else {
          setSession(null);
        }
      })
      .catch(() => !cancelled && setSession(null));
    return () => {
      cancelled = true;
    };
  }, [id]);

  if (session === undefined) {
    return <p className="text-sm text-text/60">{t("common.loading")}</p>;
  }
  if (!session) {
    return (
      <div className="flex flex-col gap-3">
        <Button variant="ghost" size="sm" onClick={onBack}>
          {t("meetings.backToList")}
        </Button>
        <p className="text-sm text-text/60">{t("meetings.notFound")}</p>
      </div>
    );
  }
  return (
    <MeetingDetail
      session={session}
      onBack={onBack}
      onSaved={onSaved}
      onDeleted={onDeleted}
    />
  );
};

export default MeetingsPage;
