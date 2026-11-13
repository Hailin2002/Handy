# Meeting live transcript follow-ups

## 2026-09-03 live Azure observation

Meeting `31ce2a98-f8ab-444a-9f54-6b46dbc27514` reproduced a confusing late transcript appearance while capture remained healthy.

- Capture started at 10:33 local time with Azure Speech and `de-DE`.
- Both sources finalized contiguous 60-second chunks without recorder or database errors.
- Azure marked the first six microphone jobs as `succeeded` but returned no transcript segments.
- The system source returned only `I'm on Mac.` in its first minute and was digitally silent from minute 2 onward.
- Byte-level verification found zero non-header bytes in system WAV chunks 2 onward; their identical SHA-256 hashes confirm exact digital silence rather than merely quiet audio.
- The system meter nevertheless remained visibly non-zero. `meetingStore.applyLevelUpdate` retains the last received peak indefinitely and neither frontend nor backend tracks a last-update timestamp or decays a stale source to zero. A loopback device that stops issuing callbacks can therefore look active while timeline gap-fill writes silence.
- At minute 7, Azure returned the first substantial microphone segments. The transcript panel appeared only then, making it look as if live transcription had suddenly started.
- Recording and transcription status are currently conflated in the UI: an empty successful STT result leaves no visible explanation even though audio capture continues safely.

## Required fix after the live meeting

- Show the transcript/status panel from meeting start, independently of whether a text segment exists.
- Display separate, explicit states for capture, queued transcription, successful empty recognition, retry/error, and latest completed audio minute.
- Explain the 60-second chunk latency in the live UI instead of presenting it as continuous streaming.
- Show per-source health so a silent system track cannot be mistaken for a failed microphone recording.
- Add per-source level freshness. Decay/reset a meter when no level event arrives within a short, tested interval, and distinguish "silent samples" from "no callbacks".
- Keep existing text visible across empty chunks and update it when the first later non-empty segment arrives.
- Do not let transcript UI state affect or stop audio capture.

## Regression and edge-case tests

- Six consecutive `succeeded` jobs with zero segments, followed by a non-empty seventh job.
- Microphone active while system capture is exact digital silence.
- A final non-zero system level followed by no further system callbacks: the meter must become stale/zero instead of remaining frozen at the last peak.
- System active while microphone is silent.
- Both sources returning segments at the same chunk boundary.
- A provider retry between two empty successful chunks.
- Meeting stopped mid-chunk: the partial minute is finalized and its eventual text appears.
- App/window navigation while capture continues: panel state reconstructs from persisted meeting state.

## Execution hold

Do not implement, build, restart Speak, replay cloud STT, or run this matrix while a live meeting is active. Preparation of this document is allowed. Execution begins only after the user explicitly confirms that the meeting has ended and asks to start.

The 2026-09-03 meeting remains local evidence. Do not copy its audio or transcript into the repository, test fixtures, logs intended for commit, or third-party systems beyond an explicitly requested Azure replay. Never print or persist its Azure credential.

## Target architecture for the fix

### 1. Separate capture, signal, persistence, and STT health

Each source (`microphone`, `system`) needs four independent states. One green state must never imply that all four are healthy.

1. **Capture callback freshness**: timestamp of the last real callback.
2. **Signal evidence**: peak, RMS, non-zero sample ratio, and whether the value is fresh, silent, or unknown.
3. **Persistence progress**: last written timeline position and last atomically finalized chunk.
4. **STT progress**: queued/running/succeeded-with-text/succeeded-empty/retrying/failed and the last covered timeline position.

The frontend must not retain a non-zero meter forever. A short UI decay may make the bar readable, but after the freshness deadline it must render zero and label the source as stale/without a current signal. A stale meter is not itself a recorder error.

### 2. Make chunk evidence auditable

Persist bounded, non-content telemetry with each finalized chunk:

- callback count;
- PCM frame/sample count;
- peak and RMS;
- non-zero sample count or ratio;
- gap-filled sample count and ratio;
- first and last callback timeline;
- device identity/config used for that chunk;
- checksum and finalized file size, which already exist.

This makes these cases distinguishable without listening to private audio:

- genuine recorded silence;
- no callbacks followed by timeline gap-fill;
- callbacks containing values that quantize to zero;
- active audio written correctly but rejected by STT;
- writer/resampler corruption.

### 3. Recover loopback capture without lying

- If system callbacks stop, immediately expire the meter instead of showing the last peak.
- When the configured/default output device changes, reopen loopback capture on the intended device and record a source-segment boundary.
- When callbacks remain absent while Windows reports a changed or invalid render endpoint, perform a bounded reopen with backoff.
- Do not continuously reopen during legitimate playback silence.
- Surface `No current system signal`, `Output device changed`, `Reconnecting`, and `System capture unavailable` as different states.
- A system-source problem must never stop or invalidate a healthy microphone track.

### 4. Make meeting STT progressive and source-aware

- Show the transcript/status area from meeting start, before the first segment.
- A successful empty STT response advances coverage but displays `No speech recognized in 00:00–01:00`; it must not look stuck.
- Keep the last valid text visible while later chunks are empty.
- Expose last finalized audio time separately from last transcribed time.
- Consider a shorter rolling STT package cadence than the 60-second persistence rollover. Persistent WAV chunking and STT latency do not need to share one boundary.
- Final stop must send the short tail package and wait for its bounded settlement without dropping a final word.

### 5. Fuse microphone and system transcripts

The current meeting can produce nearly identical Azure segments from both tracks. Presenting both verbatim duplicates the speaker. Add a deterministic fusion layer that:

- aligns segments by overlapping time ranges;
- normalizes punctuation/case only for comparison, never for stored raw evidence;
- detects near-duplicate wording with a bounded similarity threshold;
- prefers direct system capture when its signal and recognition confidence are healthy;
- keeps microphone-only local speech;
- keeps both when simultaneous speakers are genuinely different;
- never deletes raw per-source segments, only builds a fused presentation view;
- remains stable when late jobs arrive in a different order.

## Prepared test fixture set

No fixture is generated or transmitted during preparation.

### Deterministic synthetic fixtures

- exact PCM16 digital silence;
- sub-quantization float input that becomes PCM16 zero;
- low, normal, and clipped speech-like signals;
- impulse markers at every chunk boundary;
- a word split exactly across an STT package boundary;
- callbacks stopping after one non-zero frame;
- callbacks pausing and resuming with a known gap;
- different native rates: 44.1, 48, and 96 kHz;
- mono, stereo, and multi-channel downmix inputs;
- clock drift at both allowed limits and just outside them.

### Spoken reference fixtures

- German, English, and mixed German/English speech;
- long pauses, filler words, unfinished final word, and immediate stop;
- quiet speaker, distant speaker, music under speech, and overlapping speakers;
- the same speech present in both microphone and system tracks with controlled delay;
- local-only speech plus remote-only system speech;
- 30+ minute fixture with known phrase markers every minute.

### Local live-meeting regression evidence

- System minute 1 contains a short weak signal.
- System minutes 2–7 are exact PCM16 silence despite a previously non-zero UI meter.
- Azure independently returns one uncertain phrase for minute 1 and zero phrases for minutes 2–7.
- Microphone minutes 1–6 contain audio but initially receive successful empty Azure results.
- Substantial microphone text first appears in minute 7.
- Later in the same meeting, both system and microphone tracks produce substantial, almost duplicate transcripts.

## Complete edge-case matrix

### Capture and devices

- Start with both devices healthy.
- Start microphone-only by explicit choice.
- Missing microphone, missing output, and both missing during preflight.
- Default device changes before start, during start, mid-chunk, while paused, and during stop.
- Device unplug/replug with same name and with a new ID.
- Bluetooth profile changes between hands-free and stereo.
- USB device changes sample rate on reconnect.
- Windows output is muted, volume zero, paused, or has no active render stream.
- Exclusive-mode application owns the render endpoint.
- System callbacks stop after a non-zero frame while the displayed meter is active.
- Callback storm, oversized block, empty block, out-of-order timestamp, and non-finite sample.
- One source stalls while the other continues for at least two rollovers.

### Timeline, buffering, and persistence

- Exact rollover boundary and one sample before/after it.
- Stop, pause, crash, or disk failure during header write, payload write, flush, fsync, rename, checksum, and database commit.
- `.wav.part` remains header-only while buffered, then finalizes correctly.
- Queue saturation and writer slowdown never block audio callbacks.
- Timeline gap-fill is exact and explicitly measured.
- No overlap or hole between consecutive finalized ranges.
- Partial final minute is retained on normal stop and recovery.
- Two-hour soak has bounded memory and stable file count growth.
- Low disk warning and controlled stop preserve all earlier chunks.
- WAL/database busy conditions retry without losing finalized audio.

### Azure and other STT providers

- HTTP success with text, success with zero phrases, malformed success body, and partial phrase list.
- 400/401/403 configuration errors, 402/quota, 408/timeout, 413, 429 with retry-after, and 5xx.
- DNS failure, TLS failure, disconnect during upload, and response after local timeout.
- Provider returns duplicated, overlapping, reversed, or out-of-range timestamps.
- Wrong fixed language versus auto/multilingual behavior.
- Very quiet audio with bounded gain and exact silence without hallucination.
- Retry is idempotent and cannot duplicate persisted segments.
- Jobs from one source fail while the other source succeeds.
- Backlog grows and drains while capture remains unaffected.
- Tail job on stop includes the final partial word where acoustically recoverable.

### Transcript fusion

- Exact duplicate from both sources.
- Near duplicate with punctuation or one-word difference.
- Same speech delayed by 50 ms, 500 ms, 2 s, and outside the merge window.
- One source returns a truncated version.
- Two different speakers overlap in time and both must remain.
- Empty job between two valid segments.
- Late retry arrives after newer text without reordering the view incorrectly.
- Stable fused output after app navigation/reload and database resync.

### UI truthfulness and accessibility

- Transcript panel exists immediately with honest waiting status.
- Separate microphone/system meters expire independently.
- Fresh silence shows zero; stale/no-callback state is distinguishable in text.
- Last audio-secured time keeps advancing when STT is empty or offline.
- Last STT-covered time and backlog are visible and accurate.
- A source recovery updates status without erasing the warning history.
- Screen-reader announcements are restrained and name the affected source.
- Navigating away and back reconstructs current state without resetting timers or meters to false values.
- No UI operation, render error, or closed window can stop capture.

### Lifecycle and concurrency

- Rapid start/pause/resume/stop shortcut sequences.
- Stop confirmation expires and cannot target a newer meeting.
- Dictation/assistant hotkeys during a meeting obey the coordinator policy.
- App sleep/wake, lock/unlock, monitor hot-plug, and network loss/recovery.
- Closing settings or the meeting page does not close the recorder.
- Single-instance launch while recording only raises the existing app.
- Recovery after forced process termination adopts complete chunks and repairs/quarantines only the unfinished chunk.

## Execution sequence after explicit approval

1. Confirm no meeting is active. Do not infer this only from the window; verify persisted and backend state.
2. Record a read-only baseline from the current meeting: chunk continuity, hashes, signal evidence, jobs, and per-source segments.
3. Add failing unit tests for level freshness, empty-success visibility, chunk signal evidence, source recovery, and transcript fusion.
4. Implement backend signal/freshness telemetry and bounded recovery.
5. Implement the UI status model and meter expiry.
6. Implement raw-preserving dual-source transcript fusion.
7. Run deterministic unit/integration fixtures locally without cloud credentials.
8. Run targeted Azure replays only after confirming expected request count and scope.
9. Run crash/recovery, device-change, and long-soak suites.
10. Run the repository gate, verify its artifacts, build a fresh release, and only then start that exact executable.
11. Perform a short disposable live meeting with audible reference phrases on each source before using a customer meeting.

## Release acceptance criteria

- Every finalized chunk has valid RIFF/WAVE structure, expected sample count, checksum match, and continuous timeline.
- The UI never shows a fresh non-zero level beyond the freshness deadline without a new event.
- Exact digital silence cannot be presented as active system audio.
- Capture progress remains visible when STT is empty, slow, retried, or offline.
- All successful empty STT windows are explicit and time-bounded in the UI.
- Remote/system speech and local/microphone speech both appear when present.
- Near-identical dual-source recognition appears once in the fused transcript while raw evidence remains intact.
- No stop, pause, retry, recovery, or chunk boundary drops the final recoverable speech.
- Audio capture continues during provider/network failure and can be re-transcribed later.
- The targeted regression, full unit/integration suite, gate verification, release build, and disposable live smoke meeting all pass before declaring the meeting path customer-ready.
