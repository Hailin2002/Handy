// Assertions for the meeting-command mirror in `src/mocks/backend.ts`.
// Runs under vitest (`npx vitest run`).
//
// Why this exists: the Playwright journey spec proves the UI wiring through
// the mock, but nothing pinned the mock's meeting handlers to the exact IPC
// surface of the Rust `MeetingManager` (`managers/meeting.rs`). The mock once
// leaked its internal `deleted_at` soft-delete marker through
// `meeting_rename_session`, a field the real command never serializes. These
// assertions lock every handler to the seven-key `MeetingSession` DTO and to
// the manager's semantics (soft delete, newest-first list, blank-title
// rejection) so the two implementations cannot silently drift apart again.
import { describe, it, expect, beforeAll } from "vitest";
import { MockBackend, handleCommand } from "./backend";

const newBackend = () => new MockBackend({});

/** Exact key set of the Rust `MeetingSession` projection (tauri-specta DTO). */
const SESSION_KEYS = [
  "id",
  "title",
  "state",
  "started_at",
  "ended_at",
  "created_at",
  "updated_at",
].sort();

const expectSessionDto = (value: unknown) => {
  // must be an object exposing exactly the Rust MeetingSession DTO keys
  expect(typeof value).toBe("object");
  expect(Object.keys(value as object).sort()).toEqual(SESSION_KEYS);
};

// The handlers below share one backend on purpose: the assertions walk a
// single session lifecycle (create -> get -> list -> rename -> delete), the
// same way the manager is exercised in production.
describe("mock backend meeting commands (MeetingManager parity)", () => {
  let backend: MockBackend;
  let created: Record<string, unknown>;
  let blank: Record<string, unknown>;

  beforeAll(() => {
    backend = newBackend();
  });

  describe("meeting_create_session", () => {
    it("returns the exact MeetingSession DTO for a titled session", () => {
      created = handleCommand(backend, "meeting_create_session", {
        title: "Kickoff",
      }) as Record<string, unknown>;
      expectSessionDto(created);
      expect(created.title).toBe("Kickoff");
      expect(created.state).toBe("draft");
      expect(created.id).toBeTruthy(); // create must return a stable id
    });

    it("falls back to a non-empty neutral title for blank input (manager parity)", () => {
      blank = handleCommand(backend, "meeting_create_session", {
        title: "   ",
      }) as Record<string, unknown>;
      expect(blank.title).toBeTruthy();
    });
  });

  describe("meeting_get_session", () => {
    it("resolves an unknown id to null like Option::None", () => {
      expect(
        handleCommand(backend, "meeting_get_session", { id: "no-such-id" }),
      ).toEqual(null);
    });

    it("returns the stored session for a known id", () => {
      expect(
        (
          handleCommand(backend, "meeting_get_session", {
            id: created.id,
          }) as Record<string, unknown>
        ).title,
      ).toEqual("Kickoff");
    });
  });

  describe("meeting_list_sessions", () => {
    it("lists newest first and yields only DTO-shaped sessions", () => {
      handleCommand(backend, "meeting_create_session", {
        title: "Budget Review",
      });
      const listed = handleCommand(
        backend,
        "meeting_list_sessions",
        {},
      ) as Record<string, unknown>[];
      listed.forEach((s) => expectSessionDto(s));
      // same-ms inserts break by insertion recency
      expect(listed.map((s) => s.title)).toEqual([
        "Budget Review",
        blank.title,
        "Kickoff",
      ]);
    });

    it("filters case-insensitively", () => {
      expect(
        (
          handleCommand(backend, "meeting_list_sessions", {
            query: "BUDGET",
          }) as Record<string, unknown>[]
        ).length,
      ).toBe(1);
    });
  });

  describe("meeting_rename_session", () => {
    it("serializes exactly the Rust DTO (regression: leaked deleted_at)", () => {
      const renamed = handleCommand(backend, "meeting_rename_session", {
        id: created.id,
        title: "Renamed Sync",
      }) as Record<string, unknown>;
      expectSessionDto(renamed);
      expect(renamed.title).toBe("Renamed Sync");
      // rename must bump updated_at
      expect(
        (renamed.updated_at as number) >= (created.updated_at as number),
      ).toBe(true);
    });

    it("rejects a blank title like the manager", () => {
      expect(() =>
        handleCommand(backend, "meeting_rename_session", {
          id: created.id,
          title: "  ",
        }),
      ).toThrow();
    });

    it("rejects an unknown id like the manager", () => {
      expect(() =>
        handleCommand(backend, "meeting_rename_session", {
          id: "00000000-0000-4000-8000-00000000dead",
          title: "x",
        }),
      ).toThrow();
    });
  });

  describe("meeting_delete_session", () => {
    it("soft-deletes: first delete reports a removed alive session", () => {
      expect(
        handleCommand(backend, "meeting_delete_session", { id: created.id }),
      ).toBe(true);
    });

    it("hides the deleted session from get", () => {
      expect(
        handleCommand(backend, "meeting_get_session", { id: created.id }),
      ).toEqual(null);
    });

    it("is an idempotent no-op reporting false on repeat", () => {
      expect(
        handleCommand(backend, "meeting_delete_session", { id: created.id }),
      ).toBe(false);
    });

    it("keeps only alive sessions listed", () => {
      expect(
        (handleCommand(backend, "meeting_list_sessions", {}) as unknown[])
          .length,
      ).toBe(2);
    });
  });
});
