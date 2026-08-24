// Portable-installer URL resolution assertions. Runs under vitest
// (`npx vitest run`).
import { describe, it, expect } from "vitest";
import {
  resolvePortableInstallerUrl,
  PORTABLE_RELEASES_URL,
} from "./portableInstaller";

const X64_SETUP =
  "https://github.com/cjpais/Handy/releases/download/v0.9.5/Handy_0.9.5_x64-setup.exe";
const ARM64_SETUP =
  "https://github.com/cjpais/Handy/releases/download/v0.9.5/Handy_0.9.5_arm64-setup.exe";

// Trimmed copy of the real latest.json served from the updater endpoint.
const manifest = {
  version: "0.9.5",
  platforms: {
    "windows-x86_64-nsis": { url: X64_SETUP, signature: "…" },
    "windows-x86_64-msi": {
      url: "https://github.com/cjpais/Handy/releases/download/v0.9.5/Handy_0.9.5_x64_en-US.msi",
    },
    "windows-aarch64-nsis": { url: ARM64_SETUP, signature: "…" },
    "darwin-aarch64": {
      url: "https://github.com/cjpais/Handy/releases/download/v0.9.5/Handy_aarch64.app.tar.gz",
    },
  },
};

describe("resolvePortableInstallerUrl", () => {
  it("maps x64 Windows to the x64 NSIS asset pinned to the release tag", () => {
    expect(resolvePortableInstallerUrl(manifest, "windows", "x86_64")).toEqual(
      X64_SETUP,
    );
  });

  it("maps arm64 Windows to the arm64 NSIS asset", () => {
    expect(resolvePortableInstallerUrl(manifest, "windows", "aarch64")).toEqual(
      ARM64_SETUP,
    );
  });

  it("falls back to the releases page when there is no manifest", () => {
    // check() failed or returned no update
    expect(resolvePortableInstallerUrl(undefined, "windows", "x86_64")).toEqual(
      PORTABLE_RELEASES_URL,
    );
  });

  it("falls back to the releases page when no NSIS bundle exists for the arch", () => {
    expect(resolvePortableInstallerUrl(manifest, "windows", "x86")).toEqual(
      PORTABLE_RELEASES_URL,
    );
  });

  it("never offers a Windows .exe to a non-Windows portable install", () => {
    expect(resolvePortableInstallerUrl(manifest, "macos", "aarch64")).toEqual(
      PORTABLE_RELEASES_URL,
    );
  });

  it("falls back to the releases page for a malformed manifest", () => {
    expect(
      resolvePortableInstallerUrl({ platforms: "nope" }, "windows", "x86_64"),
    ).toEqual(PORTABLE_RELEASES_URL);
  });
});
