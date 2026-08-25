# Meeting Mode — Operator Readiness Checklist

**Status: PLAN-BASED READINESS CHECKLIST — NOT SHIPPED FUNCTIONALITY.**
Meeting mode does not exist in this repository yet. Per
[docs/meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)
§1, no meeting-mode production code exists under `src-tauri/src/` or
`src/components/meetings/`; [the plan](../meeting-mode-plan.md) is headed
"finale Planung, noch keine Implementierung", and both architecture contracts
cited below self-declare "spec contract — **no implementation yet**".
**Nothing in this document describes behavior an operator can observe today.**
It defines what an operator must have prepared *before* meeting mode may be
deployed responsibly, once it exists. Every item cites its plan-line or
contract-section source and carries that source's status label:

| Label | Meaning |
| --- | --- |
| `plan` | Requirement fixed by `docs/meeting-mode-plan.md` (final planning, no implementation). |
| `contract` | Normative spec contract (design-phase, changes no code). Binding on implementers, verifiable only after implementation. |
| `proposal` | Proposed i18n copy in the UX copy catalog — wording draft, nothing more. |

Audience: operators preparing a Windows deployment of Speak meeting mode
(target platform v1: Windows x64, local-first, plan §1).

---

## 1. Consent and notice obligations

An operator must be able to answer **yes** to all of these before deploying:

| # | Preparation item | Required because | Source | Status |
| --- | --- | --- | --- | --- |
| C1 | A legal review exists confirming that recording non-public conversations in the deployment jurisdiction requires the consent of everyone involved, and that meeting mode will not be positioned or used as a stealth recorder. | Plan §12: "Eine Aufnahme nichtöffentlicher Gespräche erfordert die notwendige Einwilligung … Das Feature wird nicht als Stealth-Recorder gestaltet." | plan, lines 1458–1461; non-goal "automatische oder heimliche Aufnahme", line 81 | plan |
| C2 | The pre-start consent flow is understood by all users: the start dialog shows a consent notice and `Start meeting` stays disabled until the acknowledgment is given. | Plan §12 requires a "Consent-Hinweis" before start; catalog entry P1 defines the proposed copy and the button-gating. | plan, lines 1458–1461; [catalog](../meeting-mode-ux-copy-catalog.md) P1 (keys `meeting.consent.beforeStart.{title,body,acknowledge}`) | plan + proposal |
| C3 | A visible recording indicator is guaranteed for the **entire** meeting on all surfaces (main window, overlay, tray tooltip), with text plus symbol — never color alone. | Plan §12 requires a visible indicator during the whole recording; plan §6.8 forbids color as the sole information carrier; catalog P2. | plan, lines 1458–1461 and 545–546; catalog P2 (`meeting.consent.indicator.label`) | plan + proposal |
| C4 | It is accepted that session start is fail-closed at a named consent gate: no capture resource is opened before the consent acknowledgment for this start is recorded, and refusal produces a typed error (`meeting.session.consent_gate_refused` → proposed copy family `meeting.errors.consentRequired.*`). | Session-state contract §7 fixes the choke-point seam `session_gate_consent`; error-taxonomy contract maps its refusal. | [session-state-contract](../meeting-mode-architecture/session-state-contract.md) §7; [error-taxonomy-contract](../meeting-mode-architecture/error-taxonomy-contract.md) §5 | contract |
| C5 | Users are told, before start and during processing, exactly which data leaves the device and to which provider when Azure/cloud STT or a cloud summary is selected. | Plan §12 disclosure obligation. | plan, lines 1454–1456; catalog P3–P5 (cloud notices) | plan + proposal |
| C6 | Cloud summarization is opt-in only via the persisted setting `meeting_summary_cloud_consent` (default `false`, separate from dictation settings); operators must not enable it silently and know that withdrawing it takes effect at the next stage gate, never retroactively on a running job. | Summary-provider contract §2 cloud consent gate; session-settings policy classifies the key as apply-next-session with one hot-apply exception (withdrawal). | [summary-provider-contract](../meeting-mode-architecture/summary-provider-contract.md) §2.1–§2.2; [session-settings-policy](../meeting-mode-architecture/session-settings-policy.md) §4 row 8 | contract |

## 2. Storage sizing derived from the quota contract (100 dual-track hours ≙ byte budget)

The complete byte budget derives from two plan figures plus arithmetic. Every
number below was reproduced with PowerShell (command included, so any operator
can re-run it):

```powershell
# Quota sizing (retention-quota-contract §4.2)
$perSourcePerHour = 2 * 16000 * 3600        # PCM16 bytes/s x 3600 s
$dualPerHour      = 2 * $perSourcePerHour   # mic + system track
$hard             = 100 * $dualPerHour      # AUDIO_QUOTA_HARD_BYTES
$soft             = 80  * $dualPerHour      # AUDIO_QUOTA_SOFT_BYTES
# Results: perSourcePerHour=115200000; dualPerHour=230400000;
# hard=23040000000 (~21.458 GiB); soft=18432000000 (~17.166 GiB)

# Disk-guard thresholds (disk-guard-contract §§2.1-3.1)
$chunkMax   = 32000 * 60                    # one finalized chunk per source
$stop       = 2 * $chunkMax + 1048576       # DISK_STOP_WATERMARK_BYTES
$warn       = $dualPerHour + $stop          # DISK_WARN_WATERMARK_BYTES
$hysteresis = 300 * 64000                    # five minutes of dual-track
$preflightCloud = $warn + 500000000         # cloud-STT pre-flight floor
# Results: chunkMax=1920000; stop=4888576; warn=235288576;
# hysteresis=19200000; preflightCloud=735288576
```

| # | Preparation item | Arithmetic / value | Source | Status |
| --- | --- | --- | --- | --- |
| S1 | Know the base rates: one mono track costs **115,200,000 B/h** (≈115 MB), both tracks cost **230,400,000 B/h** (≈230 MB). Derived: 2 B/sample (PCM16) × 16,000 Hz = 32,000 B/s; × 3,600 s = 115,200,000 B/h; × 2 tracks = 230,400,000 B/h. | see command above | plan §12, lines 1438–1439 (figures); [disk-guard-contract](../meeting-mode-architecture/disk-guard-contract.md) §2.1 (exact arithmetic) | plan + contract |
| S2 | Size the volume against the **hard audio quota = 100 dual-track hours ≙ 23,040,000,000 B ≈ 21.46 GiB** and the soft floor **80 h ≙ 18,432,000,000 B ≈ 17.17 GiB**. Above the projected hard value the retention engine deletes eligible audio (oldest-first, down to soft). | `100 × 230,400,000 = 23,040,000,000`; `80 × 230,400,000 = 18,432,000,000`; GiB figures = bytes ÷ 2³⁰ | [retention-quota-contract](../meeting-mode-architecture/retention-quota-contract.md) §4.2 (constants and rationale) | contract |
| S3 | Expect the start dialog to show free space and a coarse achievable duration using **3,840,000 B per dual-track minute** (`est_min = floor((free − DISK_STOP_WATERMARK_BYTES) / 3,840,000)`). Operators should sanity-check this display during rollout testing. | `230,400,000 ÷ 60 = 3,840,000` | disk-guard-contract §3.2 step 3; plan lines 1440–1442 | contract + plan |
| S4 | For cloud-STT sessions, reserve additional transient headroom of up to **500,000,000 B** for stereo upload packages (Azure file ceiling: "< 500 MB", files also < 5 h). Local-only STT adds no such reserve. | ceiling constant, no arithmetic | disk-guard-contract §2.3; plan §8, line 1113 | contract + plan |
| S5 | Ignore-as-budget but expect: manifests are capped at **1,048,576 B** (1 MiB) per session (≤ ~480 kB worst case × 2 safety factor) — relevant only when auditing per-session overhead. | ≤ 1200 chunks × ~400 B × 2 | disk-guard-contract §2.4 | contract |

Operator rule of thumb until implementation confirms real overheads: provision
free space ≳ hard quota (S2) + largest expected simultaneous cloud package
(S4) + normal non-meeting data growth, or lower the effective retention
defaults (§4) accordingly.

## 3. Disk-guard thresholds

The operator must know these thresholds exist and what each means for capacity
planning and incident response. All come from the disk-guard contract (D1–D5
are constants derived arithmetically from §2; D6 is fixed behavior) — none is
implemented yet.

| # | Threshold | Value | Derivation | Operator meaning | Source/status |
| --- | --- | --- | --- | --- | --- |
| D1 | `DISK_STOP_WATERMARK_BYTES` | 4,888,576 B | `2 × 1,920,000 + 1,048,576` (finalize both open chunks + one full manifest rewrite) | Below this free space, a running recording stops itself in a controlled way; everything captured so far stays playable. | contract ([disk-guard-contract](../meeting-mode-architecture/disk-guard-contract.md) §3.1) |
| D2 | `DISK_WARN_WATERMARK_BYTES` | 235,288,576 B | `230,400,000 + 4,888,576` (= 60 min dual-track + stop reserve) | Crossing downward guarantees ≥ 60 minutes of remaining capture before a controlled stop; a low-space warning fires here. | contract (disk-guard-contract §3.1, plan line 955 "früh warnen") |
| D3 | `DISK_PREFLIGHT_REQUIRED_FREE_BYTES` | 235,288,576 B (local STT) · 735,288,576 B (cloud STT) | D2 (+ 500,000,000 cloud reserve) | Sessions cannot start with less free space; the start button is disabled below this. **Before every deployment test day, verify ≥ 735,288,576 B free on the app-data volume if any cloud provider is configured.** | contract (disk-guard-contract §3.1, §3.2) |
| D4 | `DISK_MONITOR_INTERVAL_MS` | 30,000 ms | worst-case drift between checks = 1,920,000 B (one max chunk of one source) | No operator action; explains why warnings can lag reality by well under a minute of audio. | contract (disk-guard-contract §3.1, §3.3) |
| D5 | `DISK_WARN_CLEAR_HYSTERESIS_BYTES` | 19,200,000 B | five minutes of worst-case dual-track capture (`300 s × 64,000 B/s`). Note: the contract's §3.1 formula string ("10 × interval × rate") does **not** reproduce this number (it yields 19.2 GB); the stated five-minute rationale does. The constant value is what the contract fixes; the formula string looks like a source typo. | The low-space warning clears only above watermark + hysteresis; do not treat a lingering warning as a fault while free space hovers near the watermark. | contract (disk-guard-contract §3.1, §3.3 step 2) |
| D6 | Stop tripwire semantics | one-way latch | no upward hysteresis, no recovery within a session | A user's "Keep recording" choice cannot override the controlled stop; a new recording requires a fresh pre-flight (D3). Support scripts must not promise otherwise. | contract (disk-guard-contract §3.3 step 3) |

Incident-preparation consequence (from the same contract): a session that ends
in a disk-full state gets `ended_disk_full = true` and is then **blocked from
automatic deletion regardless of its retention policy** until a user
explicitly opens the meeting or deletes it. Operators handling "why is my
audio still there?" tickets must check this flag first.

Source: disk-guard-contract §6.2; adopted as an exemption by
[retention-quota-contract](../meeting-mode-architecture/retention-quota-contract.md)
§4.3 — both `contract`.

## 4. Retention defaults

What the operator must communicate and configure expectations around:

| # | Item | Rule | Source | Status |
| --- | --- | --- | --- | --- |
| R1 | Audio survives processing | Audio is kept through recording, transcription, and summarization; nothing is auto-deleted mid-processing. | plan §12, line 1446; retention-quota-contract §4.1 R1 (`completed_at` sweep clock) | plan + contract |
| R2 | Default is "ask, don't delete" | After the first fully processed meeting, the app asks for a standard retention rule. Options: delete immediately / 7 days / 30 days / keep permanently. | plan §12, lines 1447–1449; catalog P6 (proposed copy `meeting.retention.question.*`) | plan + proposal |
| R3 | Unanswered = inert | Without an answered selection **nothing is deleted automatically** — ever, including under quota pressure. | plan §12, line 1450; retention-quota-contract §4.3 item 1 | plan + contract |
| R4 | Processing blocks deletion | Failed or unconfirmed STT jobs prevent automatic audio deletion (snapshot-time filter **and** execution-time `RetentionGuard` immediately before the first unlink). | plan §12, lines 1451–1452; retention-quota-contract §7 double gate | plan + contract |
| R5 | Quota pressure can delete early — but only answered timed policies | When audio exceeds the hard quota (S2), eligible finished meetings are deleted oldest-first down to the soft floor; sessions that are unanswered, keep-permanent, project-protected, disk-full-flagged, or unfinished are never touched. Users are notified afterwards with real numbers. | retention-quota-contract §4.1 R2, §4.4 (normative clarification), §6.2 N1/N2 | contract |
| R6 | Project protection flag | A project boolean `protect_meeting_audio` makes all of that project's meeting audio ineligible for automatic deletion. No richer per-project quotas exist in v1 (explicit non-goal NG-2). | retention-quota-contract §4.3 item 2, §11 | contract |
| R7 | Change-log pruning | Audit rows older than **180 days** are pruned (max **1,000 rows/pass**, marker row written first); this bounds diagnostics history, not user content. | retention-quota-contract §4.2, §9 | contract |
| R8 | Imports/assets are never auto-deleted | Imported originals and media (ledger L2) are reported, never swept; deletion is user-confirm-only. Overage surfaces as a stuck-storage warning, not silent cleanup. | retention-quota-contract §2 (L2 rationale), §11 NG-6, §6.1 | contract |

## 5. Out of scope for this checklist (pointers)

- Import security duties for WhatsApp archives (untrusted input, path
  traversal, ZIP bombs): plan §12 lines 1463–1476 and
  [import-pipeline-dataflow](../meeting-mode-architecture/import-pipeline-dataflow.md) /
  [ingestion-pipeline-contract](../meeting-mode-architecture/ingestion-pipeline-contract.md).
- Backup/restore rehearsal for today's dictation data footprint:
  [backup-restore-drill](backup-restore-drill.md).
- UX copy wording details: [UX copy catalog](../meeting-mode-ux-copy-catalog.md)
  §6 (P-entries) — all proposal-only.

## 6. Evidence notes

- Byte/threshold arithmetic in §2–§3 was executed via PowerShell in this
  workspace on 2026-08-22 and matched the contract values exactly
  (`hard=23040000000`, `soft=18432000000`, `chunkMax=1920000`, `stop=4888576`,
  `warn=235288576`, `preflightCloud=735288576`, per-minute `3840000`).
  Hysteresis was verified as `300 * 64000 = 19200000`, matching the contract's
  constant; the contract's literal §3.1 formula for it does not reproduce the
  value (see D5 note).
- Contract constant values were transcribed from
  `docs/meeting-mode-architecture/disk-guard-contract.md` §2–§3.1 and
  `retention-quota-contract.md` §4.2 as read in this worktree.
- Implementation-status claims follow the index rule (index §1) and each
  source document's own status header; no code claim is made anywhere in this
  checklist.
