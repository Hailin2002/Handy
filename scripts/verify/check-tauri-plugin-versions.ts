/**
 * Tauri plugin version parity guard.
 *
 * Why: each Tauri plugin ships as a pair — a Rust crate and an npm package that
 * talk to each other over the IPC bridge. They must stay on the same
 * major.minor; tauri-bundler refuses to build otherwise.
 *
 * That refusal comes late. `cargo update` moves the Rust crates within their
 * semver ranges while package.json stays put, and nothing notices until someone
 * runs a full `tauri build` — a release-job-length wait to be told two numbers
 * disagree. This branch produced exactly that: `cargo update` took
 * tauri-plugin-dialog to 2.7.2 and tauri-plugin-fs to 2.5.1 while the npm side
 * sat at 2.6.0 and 2.4.5.
 *
 * Reading Cargo.lock rather than Cargo.toml is deliberate: the lock file holds
 * the *resolved* version, which is what actually gets compiled. A `"2.6"`
 * requirement in Cargo.toml says nothing about whether 2.7.2 is in use.
 *
 * Exit codes: 0 in sync, 1 mismatch, 2 infrastructure error.
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, "..", "..");
const CARGO_LOCK = path.join(ROOT, "src-tauri", "Cargo.lock");
const PACKAGE_JSON = path.join(ROOT, "package.json");

const colors = {
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  reset: "\x1b[0m",
};
const paint = (s: string, c: keyof typeof colors) =>
  `${colors[c]}${s}${colors.reset}`;

function fail(message: string): never {
  console.error(paint(`✗ ${message}`, "red"));
  process.exit(2);
}

/** `tauri-plugin-foo` in Cargo.lock ↔ `@tauri-apps/plugin-foo` in package.json. */
const CRATE_PREFIX = "tauri-plugin-";
const NPM_PREFIX = "@tauri-apps/plugin-";

/** Resolved crate versions, keyed by plugin name. */
function rustVersions(): Map<string, string> {
  let lock: string;
  try {
    lock = fs.readFileSync(CARGO_LOCK, "utf8");
  } catch (error) {
    fail(`cannot read Cargo.lock: ${(error as Error).message}`);
  }

  const found = new Map<string, string>();
  // Package blocks are `[[package]]` followed by name/version lines.
  for (const block of lock.split("[[package]]")) {
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(block)?.[1];
    const version = /^\s*version\s*=\s*"([^"]+)"/m.exec(block)?.[1];
    if (!name || !version || !name.startsWith(CRATE_PREFIX)) continue;
    found.set(name.slice(CRATE_PREFIX.length), version);
  }
  if (found.size === 0) {
    fail("parsed zero tauri-plugin-* crates out of Cargo.lock — the guard is broken");
  }
  return found;
}

/** Declared npm ranges, keyed by plugin name. */
function npmRanges(): Map<string, string> {
  let pkg: { dependencies?: Record<string, string>; devDependencies?: Record<string, string> };
  try {
    pkg = JSON.parse(fs.readFileSync(PACKAGE_JSON, "utf8"));
  } catch (error) {
    fail(`cannot read package.json: ${(error as Error).message}`);
  }

  const all = { ...(pkg.dependencies ?? {}), ...(pkg.devDependencies ?? {}) };
  const found = new Map<string, string>();
  for (const [name, range] of Object.entries(all)) {
    if (name.startsWith(NPM_PREFIX)) found.set(name.slice(NPM_PREFIX.length), range);
  }
  return found;
}

/** major.minor of a version or a range like `~2.7.2` / `^2.5.1`. */
function majorMinor(spec: string): string | null {
  const m = /(\d+)\.(\d+)/.exec(spec);
  return m ? `${m[1]}.${m[2]}` : null;
}

function main(): void {
  const rust = rustVersions();
  const npm = npmRanges();

  const mismatches: string[] = [];
  let compared = 0;

  for (const [plugin, npmRange] of npm) {
    const crateVersion = rust.get(plugin);
    if (!crateVersion) {
      // An npm plugin package with no Rust counterpart is dead weight, but that
      // is a different check — scripts/verify does not own dependency pruning.
      continue;
    }
    compared++;
    const a = majorMinor(crateVersion);
    const b = majorMinor(npmRange);
    if (!a || !b) {
      fail(`cannot parse versions for ${plugin}: crate "${crateVersion}", npm "${npmRange}"`);
    }
    if (a !== b) {
      mismatches.push(
        `    ${plugin}: crate tauri-plugin-${plugin} is ${crateVersion}, ` +
          `but ${NPM_PREFIX}${plugin} is ${npmRange}`,
      );
    }
  }

  if (compared === 0) {
    fail("no plugin pairs found to compare — the guard is broken");
  }

  if (mismatches.length === 0) {
    console.log(
      paint(`✓ tauri plugins: ${compared} crate/npm pair(s) on matching major.minor`, "green"),
    );
    process.exit(0);
  }

  console.error(paint("✗ tauri plugin versions disagree between Rust and npm", "red"));
  mismatches.forEach((m) => console.error(m));
  console.error(
    paint(
      "\nThe Rust crate and its npm package talk to each other; tauri-bundler\n" +
        "refuses to build when they differ. Align them, usually by bumping npm to\n" +
        "the resolved crate version:\n" +
        "    bun add @tauri-apps/plugin-<name>@~<crate version>",
      "yellow",
    ),
  );
  process.exit(1);
}

main();
