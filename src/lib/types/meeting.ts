/**
 * Frontend-side type surface for the meeting session lifecycle.
 *
 * Source of truth (read-only for this module):
 * - `docs/meeting-mode-plan.md` — "Session-Zustände", "Startvalidierung",
 *   §6.11 (laufende Ansicht), §6.12 (Processing-Checkliste);
 * - `docs/meeting-mode-architecture/session-state-contract.md` (SSC) —
 *   state set §2.1, transition table §2.2, event ordering §4;
 * - `docs/meeting-mode-architecture/live-view-streaming-contract.md` (LV) —
 *   payload schemas §2.1–2.3;
 * - `docs/meeting-mode-architecture/startup-recovery-dataflow.md` — §5
 *   recovery commands/events.
 *
 * The Rust side of these commands does not exist yet; `bindings.ts` is
 * generated and therefore only carries the five wave-1 CRUD commands. These
 * declarations describe the agreed wire shapes so the UI can be built (and
 * exercised against the mock backend) before the backend lands. Every field
 * is optional where the contract marks it optional, and consumers must ignore
 * unknown fields (LV §2 forward-compatibility rule).
 */

/** SSC §2.1 state set, verbatim. */
export type MeetingSessionState =
  | "draft"
  | "starting"
  | "recording"
  | "paused"
  | "stopping"
  | "transcribing"
  | "summarizing"
  | "completed"
  | "stopped_unprocessed"
  | "cancelled"
  | "recoverable_error";

/** Capture sources kept apart everywhere (plan §8 two-track timeline). */
export type MeetingAudioSource = "microphone" | "system";

/** Processing location as the user chooses it (plan §6.2 rule 4). */
export type MeetingProviderKind = "local" | "cloud";

export const MEETING_ACTIVE_STATES: readonly MeetingSessionState[] = [
  "starting",
  "recording",
  "paused",
  "stopping",
];

export const MEETING_PROCESSING_STATES: readonly MeetingSessionState[] = [
  "transcribing",
  "summarizing",
];

/** `meeting://session-state` (SSC §2.2, carries `state_revision` per O2). */
export interface MeetingSessionStateEvent {
  meeting_id: string;
  from?: MeetingSessionState;
  to: MeetingSessionState;
  state_revision: number;
  /** Set by T05 (mic-only fallback persisted visibly, plan Startvalidierung). */
  microphone_only?: boolean;
}

/** `meeting://session-error` (SSC T04/T13/T17). Messages are pre-redacted (O7). */
export interface MeetingSessionErrorEvent {
  meeting_id: string;
  /** Taxonomy code, e.g. `micLost`, `diskFull`, `cloudUnreachable`. */
  code: string;
  /** Already-redacted detail; never a raw backend error (plan §6.2 rule 8). */
  detail?: string;
  state_revision?: number;
  /** False ⇒ the session cannot continue without user action. */
  recoverable?: boolean;
}

/** `meeting://level-update` (LV §2.3). */
export interface MeetingLevelUpdateEvent {
  meeting_id: string;
  source: MeetingAudioSource;
  peak: number;
  rms: number;
  at_timeline_ms?: number;
}

/** One persisted, finished transcript segment (LV §2.1). */
export interface MeetingTranscriptSegment {
  id: string;
  job_id?: string;
  source: MeetingAudioSource;
  start_ms: number;
  end_ms: number;
  sequence: number;
  text: string;
  confidence?: number;
}

/** `meeting://transcript-delta` (LV §2.1) — append-only. */
export interface MeetingTranscriptDeltaEvent {
  meeting_id: string;
  segments: MeetingTranscriptSegment[];
  last_sequence: number;
  state_revision?: number;
  truncated?: boolean;
}

/** Per-source job counters (LV §2.2). */
export interface MeetingJobCounts {
  queued: number;
  running: number;
  retry_wait: number;
  failed: number;
  succeeded: number;
}

/** `meeting://jobs-updated` (LV §2.2) — aggregates only, never per-job churn. */
export interface MeetingJobsUpdatedEvent {
  meeting_id: string;
  per_source: Partial<Record<MeetingAudioSource, MeetingJobCounts>>;
  total_backlog: number;
  oldest_failed?: { count: number; first_failed_at?: string };
  state_revision?: number;
  /** Checklist totals (§6.12); part of the snapshot, echoed when known. */
  eligible_total?: number;
  succeeded_total?: number;
  prepared_total?: number;
  prepared_done?: number;
}

/** Why a session was interrupted (startup-recovery-dataflow §3 S7). */
export type MeetingRecoveryCause = "crash" | "sleep_wake" | "unknown";

/** One pending recovery offer (`meeting_list_recoveries`). */
export interface MeetingRecoveryOffer {
  meeting_id: string;
  title: string;
  cause: MeetingRecoveryCause;
  recovered_chunks: number;
  damaged_chunks: number;
  quarantined_files: number;
  interrupted_at?: string;
}

/** `meeting://recovery-report` (startup-recovery-dataflow §5 U1/U6). */
export interface MeetingRecoveryReportEvent {
  offers: MeetingRecoveryOffer[];
}

/** How the user resolves a recovery offer (U2–U4). */
export type MeetingRecoveryResolution = "recover" | "later" | "delete";

/** One row of the compact start dialog (plan §6.7 / "Startvalidierung"). */
export type MeetingPreflightCheckId =
  | "microphone"
  | "systemAudio"
  | "transcription"
  | "language"
  | "disk"
  | "permission";

export type MeetingPreflightStatus = "ok" | "warning" | "error";

export interface MeetingPreflightCheck {
  id: MeetingPreflightCheckId;
  status: MeetingPreflightStatus;
  /** Human-readable current value ("Default microphone", "Deutsch", …). */
  value?: string;
  /** Error taxonomy code so the UI can pick the repair copy (§6.2 rule 8). */
  code?: string;
  /** Pre-redacted detail line. */
  detail?: string;
}

export interface MeetingPreflightReport {
  checks: MeetingPreflightCheck[];
  /** Plan: the start button is only enabled once this is true. */
  can_start: boolean;
  /** Loopback unavailable ⇒ the user may explicitly fall back (plan E5). */
  microphone_only_available?: boolean;
}

/**
 * `get_meeting_snapshot` (SSC §5 / LV §6.2). The UI is a pure function of this
 * snapshot plus subsequent events; nothing about a session is persisted in the
 * frontend (SSC §5 rule 1, FSC B4).
 */
export interface MeetingSnapshot {
  meeting_id: string;
  title: string;
  state: MeetingSessionState;
  state_revision: number;
  /** Milliseconds of capture already recorded (pause-corrected). */
  elapsed_ms: number;
  /** Epoch ms of the moment capture last resumed; null while not capturing. */
  running_since?: number | null;
  microphone_only?: boolean;
  provider_kind: MeetingProviderKind;
  provider_name?: string | null;
  summary_enabled?: boolean;
  transcript_head_sequence?: number;
  jobs?: MeetingJobsUpdatedEvent;
  levels?: Partial<Record<MeetingAudioSource, number>>;
  last_error?: { code: string; detail?: string } | null;
}

/** Arguments accepted by `start_meeting` (plan §6.7 session snapshot). */
export interface MeetingStartRequest {
  meetingId: string;
  title: string;
  microphoneOnly: boolean;
  summaryEnabled: boolean;
  consentAcknowledged: boolean;
}
