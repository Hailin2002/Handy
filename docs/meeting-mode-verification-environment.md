# Meeting Mode — Windows Verification-Environment Constraints

Status: reference document for every gate that interprets verification results on
the primary Windows verification machine (Windows, agent shells running
Windows PowerShell via the DSH harness).

**Purpose:** prevent future verifiers from misreading _environment_ failures as
_product_ failures. Every fact below lists the observed symptom, the correct
fallback command, and a citation to a committed file/line or a recorded log so
the next verifier can reproduce the diagnosis instead of re-deriving it.

**How to read the classification tags:**

| Tag               | Meaning                                                                                                                                           |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| **machine-local** | True only on this verification machine / its checkout configuration. Other contributors will not see it; do not "fix" product code because of it. |
| **repo-enforced** | Committed into this repository and therefore active on every machine. Changing it requires a deliberate repo change (and re-verification).        |
| **harness-local** | True inside the sandboxed agent shell only. A normal user terminal does not have this constraint.                                                 |

---

## F1 — `bun` is not installed

**Classification:** machine-local

**Observed symptom (reproduced 2026-08-22):**

```text
PS> bun --version
bun : The term 'bun' is not recognized as the name of a cmdlet, function,
script file, or operable program...
```

Every `bun run <script>` invocation from AGENTS.md/BUILD.md therefore fails
before any code runs. This is **not** a repository problem: `bun.lock`,
`package.json`, and all scripts are intact, and `node_modules` is fully
installed.

**Fallback commands (all verified runnable in this session):**

```powershell
# Script equivalents (package.json:11-20):
npm.cmd run lint                 # = bun run lint
npm.cmd run check:translations   # verified: exit 0, "All 23 languages have complete translations!"
node_modules\.bin\eslint.cmd src                  # verified: eslint v9.39.1
node_modules\.bin\prettier.cmd --check <paths>    # verified, see F3
node_modules\.bin\vite.cmd --version              # verified: vite/6.4.1, exit 0
node_modules\.bin\tauri.cmd --version             # verified: tauri-cli 2.11.4, exit 0
node_modules\.bin\playwright.cmd test             # exists (node_modules/.bin/playwright.cmd); full run not re-executed in-session
```

Tool availability confirmed by directory listing of `node_modules\.bin`
(`prettier`, `eslint`, `vite`, `tauri`, `playwright` + `.cmd` shims all present).

**Citations:**

- Session observation 2026-08-22: `Get-Command bun` fails; `node --version` → v24.19.0.
- `package.json:6-21` — script definitions used by the fallbacks above.

---

## F2 — PowerShell execution policy blocks `npm.ps1`

**Classification:** machine-local

**Observed symptom (reproduced 2026-08-22):**

```text
PS> npm run lint
npm : File C:\Program Files\nodejs\npm.ps1 cannot be loaded because running
scripts is disabled on this system. For more information, see about_Execution_Policies
```

The bare `npm` alias resolves to the PowerShell shim `npm.ps1`, which the
machine's execution policy refuses to load — so even with node/npm installed,
plain `npm run …` fails in pwsh before npm itself starts.

**Fallback command (verified):** call the CMD shim explicitly:

```powershell
npm.cmd run check:translations   # verified exit 0
npm.cmd --version                # verified 11.17.0
```

**Citations:**

- Session observation 2026-08-22 (stderr quoted above).
- `npm.ps1` execution-policy denial; `npm.cmd` works.

---

## F3 — `core.autocrlf=true` makes `prettier --check .` report CRLF-only failures

**Classification:** machine-local git checkout setting; the repo side is
repo-enforced (`.prettierrc` pins LF)

**Observed symptom (reproduced 2026-08-22):** `prettier --check` fails on files
whose committed content is already correctly formatted:

```text
[warn] docs/meeting-mode-plan.md
[warn] src/components/ui/Textarea.tsx
[warn] src/i18n/locales/de/translation.json
[warn] src/i18n/locales/en/translation.json
Code style issues found in 4 files. Run Prettier with --write to fix.
```

Identical list recorded in the phase-0 baseline
(`docs/meeting-mode-phase0-baseline.md:84-94`, §2.1) — the failures are stable
across runs because they are produced by checkout, not by content drift.

**Diagnosis (verified in this session):**

- `git config core.autocrlf` → `true`: the working tree is checked out with CRLF.
- Working-tree `src/components/ui/Textarea.tsx` contains 27 CRLF pairs; the
  committed blob (`git show HEAD:…`) contains 0.
- `.prettierrc:2` sets `"endOfLine": "lf"`, so any CRLF file fails `--check`
  regardless of its actual formatting.
- Writing the same file content with CRLF normalized to LF to a temp file and
  running `prettier --check` on it passes: `All matched files use Prettier
code style!`, exit 0 — and prettier's reformatted output is line-identical
  to the input (empty `Compare-Object`).

**Interpretation rule:** a prettier failure on an untouched, committed-clean
file under `autocrlf=true` is checkout noise, **not** a formatting regression.
Never "fix" it with `prettier --write` — that would rewrite real line endings
into the diff.

**Fallback command (verified):** blob-level/targeted check on LF-normalized
content:

```powershell
$p   = 'src/components/ui/Textarea.tsx'   # any suspect file
$lf  = [IO.File]::ReadAllText((Resolve-Path $p)).Replace("`r`n", "`n")
$tmp = Join-Path $env:TEMP ("prettier-lf-check" + [IO.Path]::GetExtension($p))
[IO.File]::WriteAllText($tmp, $lf, [Text.UTF8Encoding]::new($false))
node_modules\.bin\prettier.cmd --check $tmp   # exit 0 => blob formatting is fine
```

For triage over many files, run targeted checks on the flagged paths instead of
the whole tree (`prettier --check .` also scans `vendor/` and large docs — no
ignore entry in `.prettierignore` — and exceeded a 180 s sandbox-shell timeout
on 2026-08-22).

**Citations:**

- `.prettierrc:2` — `"endOfLine": "lf"` (repo-enforced).
- Local git config `core.autocrlf=true` (session observation).
- `docs/meeting-mode-phase0-baseline.md:60-62,84-94` — original baseline
  measurement and identical warn list.

---

## F4 — `transcribe-cpp-sys` native build is pinned to serial execution

**Classification:** **repo-enforced**

Any MSBuild worker parallelism (`/m`, even `/m:1`) makes the cmake-driven
native build fail silently on this machine (MSBuild worker nodes die without
diagnostics; plus FileTracker `MAX_PATH` overflow, FTK1011), while a serial
invocation succeeds. The enforcement lives in the repo, so every cargo
invocation from a repository checkout builds serially:

- `.cargo/config.toml:3-11` — `[env] CMAKE_BUILD_PARALLEL_LEVEL = "1"` with the
  rationale comment ("any MSBuild worker parallelism … makes the cmake-driven
  native build fail silently here").
- `src-tauri/Cargo.toml:194-202` — `[patch.crates-io]` replaces `cmake` with
  `path = "../vendor/cmake"`.
- `vendor/cmake/src/lib.rs:896-906` — local patch honors
  `CMAKE_BUILD_PARALLEL_LEVEL` ahead of `NUM_JOBS` (cargo always overwrites
  `NUM_JOBS` for build scripts, rust-lang/cargo#9972).
- `vendor/cmake/src/lib.rs:1177-1251` — short NTFS-junction relocation of long
  Windows build dirs (>120 chars) to defuse FileTracker MAX_PATH.
- Contributor-facing write-up (landed with task-0045):
  `BUILD.md:267-330` ("Windows native build fails silently inside MSBuild …
  serial build enforced"), including the serial-vs-parallel differential
  measurement at `BUILD.md:278`.

**Correct verifier behavior:** no fallback needed — plain `cargo test` /
`cargo clippy` from `src-tauri` inherit the pin. Do **not** raise
`CMAKE_BUILD_PARALLEL_LEVEL` (an env override would defeat the pin;
`.cargo/config.toml:11`) without re-verifying MSBuild parallelism. Sanity
check before interpreting a native-build failure:
`Get-ChildItem Env:CMAKE_BUILD_PARALLEL_LEVEL` should print `1`
(see `BUILD.md:368-369`). Baseline evidence of the serial build passing:
`src-tauri/target/baseline-tools/logs/11-cargo-test-serial-fix.log`
(cited in `docs/meeting-mode-phase0-baseline.md:71,192`; note that the log
files themselves live outside the repo and are absent from fresh worktrees —
`src-tauri/target` did not exist in this worktree, checked 2026-08-22).

**Classification note:** the _pin_ is repo-enforced; the underlying MSBuild
defect it works around is machine-local.

---

## F5 — `VULKAN_SDK` may be missing in restricted/fresh shells

**Classification:** machine-local (SDK install + which shell inherits it)

A fresh CMake configure pass of `transcribe-cpp-sys` needs `VULKAN_SDK` (or
`glslc` on PATH); without it `FindVulkan` fails with `Could NOT find Vulkan`.
On this machine the SDK is registered machine-wide
(`C:\VulkanSDK\1.4.357.0`), but restricted shells (e.g. the sandbox harness)
do not always inherit the machine environment — the independent verification
run in the phase-0 baseline had to set it explicitly
(`docs/meeting-mode-phase0-baseline.md:78-80`).

**Observed variance:** a normal user terminal inherits the variable; this
session's harness shell also happened to inherit it (`$env:VULKAN_SDK` →
`C:\VulkanSDK\1.4.357.0`; registered at **Machine** scope only — the User
scope value is empty — re-checked 2026-08-22). Absence is therefore
shell-dependent — check before blaming the build.

**Fallback command** (documented; matches BUILD.md except SDK version):

```powershell
$env:VULKAN_SDK = 'C:\VulkanSDK\1.4.357.0'   # adjust to installed SDK version
$env:PATH      = "$env:VULKAN_SDK\Bin;$env:PATH"
```

Not re-executed end-to-end in-session (would require a fresh native cmake
configure); taken verbatim from the cited sources.

**Citations:**

- `BUILD.md:53` — "Open a new terminal afterward so `VULKAN_SDK` is set."
- `BUILD.md:328-337` — Vulkan SDK requirement + explicit-set snippet.
- `BUILD.md:264` — cross-reference from the serial-build section.
- `docs/meeting-mode-phase0-baseline.md:174-180` — original observation:
  restricted shells do not inherit the machine environment, `FindVulkan`
  otherwise fails with `Could NOT find Vulkan`.

---

## F6 — Sandboxed shells: spawning helper processes over stdio pipes fails with EPERM

**Classification:** harness-local (DSH process sandbox), not present in a
normal user terminal

**Observed symptom (reproduced live 2026-08-22):**

```text
PS> node -e "const {spawnSync}=require('child_process');
     const r=spawnSync(process.execPath,['-e','console.log(1)'],{stdio:'pipe'});
     console.log('status:',r.status,'error:',r.error && r.error.code)"
status: null error: EPERM
```

Inside the sandboxed shell, programs cannot open named pipes, so any tool that
launches a helper binary and captures its output through stdio pipes fails with
`EPERM` (status `null`). Affected patterns include esbuild's JS API (spawns the
esbuild service binary over pipes — surfaces as `vite dev/build` dying with
EPERM when launched from an agent shell) and MSBuild worker invocations that
capture nested tool output. This is a sandbox boundary, **not** a defect of the
toolchain or the app.

**Fallback behavior:**

- Prefer tool invocations whose output goes to the terminal or a file rather
  than through captured pipes (`stdio: 'inherit'|'ignore'` equivalents);
  PowerShell's own pipelines are unaffected.
- Otherwise run the affected command in a non-sandboxed shell; per the DSH
  sandbox policy the pipe-capture restriction does not apply there.

**Citations:**

- Live reproduction quoted above (2026-08-22, this session).
- DSH harness runtime policy: programs cannot open named pipes;
  `child_process.spawn`/`exec` with default `stdio: 'pipe'` fails with EPERM,
  while `stdio: 'inherit'` and `'ignore'` work.

---

## F7 — `%LOCALAPPDATA%\tcs` junction creation can be permission-denied in restricted shells

**Classification:** machine-local, best-effort handled by the vendored patch

`transcribe-cpp-sys` creates its own short-path junction under
`%LOCALAPPDATA%\tcs\<hash>`; in restricted shells this can fail with
"Access is denied". The vendored cmake patch then falls back to building in the
deep `OUT_DIR`, kept safe by its own junction mechanism
(`vendor/cmake/src/lib.rs:1224-1251` emits
`cargo:warning=cmake-rs: could not create short build junction …` /
`using short build junction …`). Both paths were verified during the phase-0
baseline.

**Verifier interpretation:** a `could not create short build junction` cargo
warning is degraded-but-working, not fatal; a _fatal_ native-build failure
should still be diagnosed against F4/F5 first.

**Citations:** `docs/meeting-mode-phase0-baseline.md:181-186`;
`vendor/cmake/src/lib.rs:1224-1251`.

---

## Cross-fact notes for gate interpretation

1. **Log availability:** several citations point to logs under
   `src-tauri/target/baseline-tools/logs/`. That directory is untracked build
   output and does **not** exist in fresh worktrees (verified 2026-08-22,
   `Test-Path src-tauri\target` → False); treat the citing documents as the
   durable record.
2. **Full-tree commands are timeout-prone here:** `prettier --check .` exceeded
   a 180 s sandbox-shell timeout on 2026-08-22; use targeted checks (F3).
3. **Decision rule:** if a failure matches one of F1–F7's symptoms in the cited
   location, classify it as environment noise, apply the fallback, and record
   that classification in the verification log — do not raise product bugs
   from it. If a failure matches none of these facts, it is a candidate
   genuine defect and must be investigated normally.

## Non-goals

This document deliberately does **not**: propose fixing the upstream
MSBuild-parallelism or FileTracker bugs, recommend changing `core.autocrlf` or
execution-policy settings machine-wide, or relax any repo-enforced pin (F4).
Those are machine/toolchain concerns owned outside meeting-mode docs.
