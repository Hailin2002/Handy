import { test, expect } from "@playwright/test";
import { installTauriMock, tEn, readBridge } from "./helpers/tauri-mock";

/**
 * Full recording journey (plan 6.7, 6.11, 6.12) against the mocked
 * `MeetingManager`: start validation -> start -> live timer, level meters and
 * transcript deltas -> pause -> resume -> stop with confirmation -> the honest
 * processing checklist -> the finished meeting.
 *
 * Everything asserted here is driven by the `meeting://*` events the mock
 * publishes with contract-shaped payloads and gap-free `state_revision`s, so
 * the spec exercises the same store path production will use.
 */
const openMeetings = async (page: import("@playwright/test").Page) => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: tEn("nav.meetings") })
    .click();
};

test.describe("Meeting recording journey (mocked Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");
  });

  test("preflight gates the start, then the full record and process flow runs", async ({
    page,
  }) => {
    await openMeetings(page);
    await page
      .getByRole("button", { name: tEn("meetings.newMeeting") })
      .click();

    // --- start validation ("Startvalidierung") ---------------------------
    const startButton = page.getByRole("button", {
      name: tEn("meeting.actions.startMeeting"),
    });
    await expect(startButton).toBeVisible();
    // Every check renders a plain label and a status word, never a code.
    const micRow = page.locator("[data-preflight-row='microphone']");
    await expect(
      micRow.getByText(tEn("meeting.preflight.row.microphone"), {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      micRow.getByText(tEn("meeting.preflight.status.ok"), { exact: true }),
    ).toBeVisible();
    await expect(
      page
        .locator("[data-preflight-row='systemAudio']")
        .getByText(tEn("meeting.preflight.row.systemAudio"), { exact: true }),
    ).toBeVisible();
    await expect(page.locator("[data-preflight-row='disk']")).toHaveAttribute(
      "data-preflight-status",
      "ok",
    );

    // Consent is required before the first byte is captured (plan 12 / P1).
    await expect(startButton).toBeDisabled();
    await page
      .getByLabel(tEn("meeting.consent.beforeStart.acknowledge"))
      .check();
    await expect(startButton).toBeEnabled();

    await startButton.click();

    // --- live recording view (plan 6.11) ---------------------------------
    await expect(
      page.getByText(tEn("meeting.consent.indicator.label")),
    ).toBeVisible();
    await expect(
      page.locator("[data-meeting-badge='recording']"),
    ).toBeVisible();
    await expect(
      page.getByText(tEn("meeting.session.recording.saved")),
    ).toBeVisible();

    // The duration clock runs off the backend-provided start time.
    await expect(
      page.locator("[aria-label^='Recording duration']"),
    ).toBeVisible();

    // Separate microphone and system meters, each with a text label, moving
    // off zero once level events arrive.
    const micMeter = page.getByRole("meter", {
      name: tEn("meeting.live.levels.microphone"),
    });
    const systemMeter = page.getByRole("meter", {
      name: tEn("meeting.live.levels.system"),
    });
    await expect(micMeter).toBeVisible();
    await expect(systemMeter).toBeVisible();
    await expect
      .poll(async () => Number(await micMeter.getAttribute("aria-valuenow")))
      .toBeGreaterThan(0);

    // Finished transcript segments appear with timestamps.
    await expect(
      page.getByText("Welcome everyone, let us start with the budget review."),
    ).toBeVisible();

    // --- pause / resume (plan "Pause") -----------------------------------
    await page
      .getByRole("button", { name: tEn("meeting.actions.pause") })
      .click();
    await expect(page.locator("[data-meeting-badge='paused']")).toBeVisible();
    await expect(
      page.getByText(tEn("meeting.session.paused.body")),
    ).toBeVisible();

    await page
      .getByRole("button", { name: tEn("meeting.actions.resume") })
      .click();
    await expect(
      page.locator("[data-meeting-badge='recording']"),
    ).toBeVisible();

    // --- stop behind an explicit confirmation (6.15 release gate) --------
    await page
      .getByRole("button", { name: tEn("meeting.actions.endMeeting") })
      .click();
    const stopDialog = page.getByRole("dialog");
    await expect(stopDialog).toBeVisible();
    await expect(
      stopDialog.getByRole("button", {
        name: tEn("meeting.actions.keepRecording"),
      }),
    ).toBeVisible();
    await stopDialog
      .getByRole("button", { name: tEn("meeting.actions.endMeeting") })
      .click();

    // --- processing checklist (plan 6.12) --------------------------------
    await expect(
      page.getByText(tEn("meeting.processing.step.saved")),
    ).toBeVisible();
    await expect(page.locator("[data-step='transcribed']")).toBeVisible();
    // The stages are separate and honest: audio saved is done long before
    // transcription or summarisation are.
    await expect(page.locator("[data-step='saved']")).toHaveAttribute(
      "data-step-state",
      "done",
    );
    await expect
      .poll(async () =>
        page.locator("[data-step='summary']").getAttribute("data-step-state"),
      )
      .toBe("done");
    await expect(
      page.getByText(tEn("meeting.session.completed.saved")),
    ).toBeVisible();

    // --- back into the library via the finished meeting -------------------
    await page
      .getByRole("button", { name: tEn("meeting.actions.openMeeting") })
      .click();
    await expect(
      page.getByRole("textbox", { name: tEn("meetings.titleInputLabel") }),
    ).toBeVisible();

    // The lifecycle crossed the IPC bridge in the contract's order.
    const snapshot = await readBridge(page);
    const lifecycle = snapshot.invocations
      .map((i) => i.command)
      .filter((command) =>
        [
          "preflight_meeting",
          "start_meeting",
          "pause_meeting",
          "resume_meeting",
          "stop_meeting",
        ].includes(command),
      )
      // React StrictMode invokes mount effects twice in dev, so the preflight
      // read can repeat. Repeats of a read are harmless; the *order* of the
      // lifecycle is what this asserts.
      .filter((command, index, all) => command !== all[index - 1]);
    expect(lifecycle).toEqual([
      "preflight_meeting",
      "start_meeting",
      "pause_meeting",
      "resume_meeting",
      "stop_meeting",
    ]);
  });

  test("the sidebar shows the running meeting while recording", async ({
    page,
  }) => {
    await openMeetings(page);
    await page
      .getByRole("button", { name: tEn("meetings.newMeeting") })
      .click();
    await page
      .getByLabel(tEn("meeting.consent.beforeStart.acknowledge"))
      .check();
    await page
      .getByRole("button", { name: tEn("meeting.actions.startMeeting") })
      .click();

    // Plan 6.2 rule 9: the recording state is consistent across surfaces.
    // The pinned nav entry is derived from the same store as the recorder.
    await expect(
      page
        .getByRole("navigation")
        .getByRole("button", { name: /Recording . / }),
    ).toBeVisible();
  });
});

/**
 * Crash-recovery flow (plan "Session-Zustaende";
 * docs/meeting-mode-architecture/startup-recovery-dataflow.md 5, U1-U4).
 *
 * A session interrupted by a crash must never disappear silently: the audio
 * captured so far is offered for processing through a visible banner with
 * three named outcomes.
 */
test.describe("Meeting recovery banner (mocked Tauri IPC)", () => {
  const OFFER = {
    meeting_id: "recovered-1",
    title: "Interrupted Standup",
    cause: "crash" as const,
    recovered_chunks: 4,
    damaged_chunks: 1,
    quarantined_files: 0,
  };

  test.beforeEach(async ({ page }) => {
    await installTauriMock(page, { meetingRecoveries: [OFFER] });
    await page.goto("/");
    await openMeetings(page);
  });

  test("offers an interrupted recording for processing, even on an empty library", async ({
    page,
  }) => {
    const banner = page.locator("[data-recovery-offer='recovered-1']");
    await expect(banner).toBeVisible();
    await expect(banner.getByText(tEn("meeting.recovery.title"))).toBeVisible();
    // The banner names the meeting and the interruption cause in plain words.
    await expect(banner.getByText(/Interrupted Standup/)).toBeVisible();
    await expect(
      banner.getByText(tEn("meeting.recovery.causeCrash")),
    ).toBeVisible();
    // It coexists with the first-run empty state instead of being hidden.
    await expect(
      page.getByRole("button", { name: tEn("meetings.newMeeting") }),
    ).toBeVisible();

    // All three outcomes are reachable without hover (6.14).
    await expect(
      banner.getByRole("button", { name: tEn("meeting.actions.decideLater") }),
    ).toBeVisible();
    await expect(
      banner.getByRole("button", {
        name: tEn("meeting.actions.deleteRecording"),
      }),
    ).toBeVisible();

    await banner
      .getByRole("button", { name: tEn("meeting.actions.recover") })
      .click();
    await expect(banner).toBeHidden();

    const snapshot = await readBridge(page);
    expect(
      snapshot.invocations.some(
        (i) =>
          i.command === "meeting_resolve_recovery" &&
          i.args.meetingId === "recovered-1" &&
          i.args.resolution === "recover",
      ),
    ).toBe(true);
  });
});
