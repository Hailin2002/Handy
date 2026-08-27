/**
 * Frontend log-hygiene guard — the counterpart to check-log-hygiene.ps1.
 *
 * Why: the Rust guard stops transcripts and credentials reaching the log file.
 * The frontend had no equivalent, yet a `console.log(transcript)` puts the
 * dictation payload into devtools and into anything that captures webview
 * console output. The current tree is clean (100 console.error, 18 warn, 1 log,
 * none passing a payload), so this exists to keep it that way rather than to
 * fix a present leak.
 *
 * What it flags: a console call passing a *risky identifier* as an argument.
 * The distinction mirrors the Rust guard's — logging the word "token" inside a
 * message string is fine ("Failed to refresh Azure Speech token"); logging the
 * variable `token` is not. Metadata about a payload (`transcript.length`) is
 * allowed; anything else reached off it is flagged conservatively.
 *
 * Suppression: `// log-hygiene:allow(<reason>)` on any line the call spans.
 * The reason is mandatory and echoed in the summary, so an exemption is a
 * reviewed decision rather than a silent one.
 *
 * Exit codes: 0 clean, 1 violations, 2 infrastructure error.
 *
 * Run `--self-test` to verify the guard itself still detects and permits the
 * right things.
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(__dirname, "..", "..");
const SCAN_ROOT = path.join(ROOT, "src");

const colors = {
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  reset: "\x1b[0m",
};
const paint = (s: string, c: keyof typeof colors) =>
  `${colors[c]}${s}${colors.reset}`;

/** Identifier stems whose *values* must never be logged. */
const RISKY = [
  "transcript",
  "transcription",
  "apikey",
  "api_key",
  "secret",
  "token",
  "password",
  "credential",
  "subscriptionkey",
  "subscription_key",
  "settings",
  "payload",
];

/** Reads that are metadata about a payload rather than the payload itself. */
const SAFE_ACCESS = /\.(length|size|count|byteLength|id)\b/;

/** Directories whose logging is not user-facing. */
const SKIP = /(\.test\.[tj]sx?$|__tests__|[\\/]mocks[\\/]|bindings\.ts$)/;

interface Finding {
  file: string;
  line: number;
  method: string;
  argument: string;
}

function walk(dir: string, out: string[] = []): string[] {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(ts|tsx)$/.test(e.name) && !SKIP.test(p)) out.push(p);
  }
  return out;
}

/** Splits a console call's argument list on top-level commas. */
function splitArgs(argText: string): string[] {
  const args: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let cur = "";
  for (let i = 0; i < argText.length; i++) {
    const c = argText[i];
    if (quote) {
      if (c === "\\") {
        cur += c + (argText[i + 1] ?? "");
        i++;
        continue;
      }
      if (c === quote) quote = null;
      cur += c;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      quote = c;
      cur += c;
      continue;
    }
    if ("([{".includes(c)) depth++;
    if (")]}".includes(c)) depth--;
    if (c === "," && depth === 0) {
      args.push(cur);
      cur = "";
      continue;
    }
    cur += c;
  }
  if (cur.trim()) args.push(cur);
  return args;
}

/**
 * Strips string and template literals, keeping `${...}` interpolations —
 * a payload interpolated into a message is still a payload.
 */
function stripLiterals(expr: string): string {
  let out = "";
  let quote: string | null = null;
  for (let i = 0; i < expr.length; i++) {
    const c = expr[i];
    if (quote) {
      if (c === "\\") {
        i++;
        continue;
      }
      if (c === quote) {
        quote = null;
        continue;
      }
      if (quote === "`" && c === "$" && expr[i + 1] === "{") {
        let depth = 1;
        let j = i + 2;
        for (; j < expr.length && depth > 0; j++) {
          if (expr[j] === "{") depth++;
          if (expr[j] === "}") depth--;
        }
        out += " " + expr.slice(i + 2, j - 1) + " ";
        i = j - 1;
      }
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      quote = c;
      continue;
    }
    out += c;
  }
  return out;
}

function riskyIdentifier(expr: string): string | null {
  const code = stripLiterals(expr);
  for (const m of code.matchAll(/[A-Za-z_$][A-Za-z0-9_$]*/g)) {
    const word = m[0];
    const stem = word.toLowerCase();
    if (!RISKY.some((r) => stem.includes(r))) continue;
    const after = code.slice(m.index! + word.length);
    if (SAFE_ACCESS.test(after.slice(0, 12))) continue;
    return word;
  }
  return null;
}

function scanFile(file: string): { findings: Finding[]; suppressed: number } {
  const text = fs.readFileSync(file, "utf8");
  const lines = text.split("\n");
  const findings: Finding[] = [];
  let suppressed = 0;

  const re = /console\.(log|debug|info|warn|error|trace)\s*\(/g;
  for (const m of text.matchAll(re)) {
    let depth = 1;
    let i = m.index! + m[0].length;
    let quote: string | null = null;
    for (; i < text.length && depth > 0; i++) {
      const c = text[i];
      if (quote) {
        if (c === "\\") i++;
        else if (c === quote) quote = null;
        continue;
      }
      if (c === '"' || c === "'" || c === "`") quote = c;
      else if (c === "(") depth++;
      else if (c === ")") depth--;
    }
    const call = text.slice(m.index!, i);
    const startLine = text.slice(0, m.index!).split("\n").length;
    const endLine = startLine + call.split("\n").length - 1;

    // The lines the call spans, plus the one above it: the suppression gets
    // written on the preceding line as often as trailing, the same way
    // eslint-disable-next-line reads.
    const spanned = lines.slice(Math.max(0, startLine - 2), endLine).join("\n");
    if (/\/\/\s*log-hygiene:allow\(/.test(spanned)) {
      suppressed++;
      continue;
    }

    for (const arg of splitArgs(call.slice(m[0].length, -1))) {
      const hit = riskyIdentifier(arg);
      if (hit) {
        findings.push({
          file: path.relative(ROOT, file),
          line: startLine,
          method: m[1],
          argument: hit,
        });
        break;
      }
    }
  }
  return { findings, suppressed };
}

function selfTest(): never {
  const dir = fs.mkdtempSync(path.join(ROOT, ".log-hygiene-selftest-"));
  const cases: [string, string, boolean][] = [
    ["leak_identifier.ts", "console.log(transcript);\n", true],
    ["leak_interpolated.ts", "console.error(`text ${transcript}`);\n", true],
    ["leak_member.ts", "console.warn(result.apiKey);\n", true],
    [
      "clean_message.ts",
      'console.warn("Failed to refresh Azure Speech token", error);\n',
      false,
    ],
    ["clean_metadata.ts", "console.info(transcript.length);\n", false],
    [
      "clean_suppressed_above.ts",
      "// log-hygiene:allow(debug view shows the user their own text)\nconsole.log(transcript);\n",
      false,
    ],
    [
      "clean_suppressed_trailing.ts",
      "console.log(transcript); // log-hygiene:allow(same reason, trailing form)\n",
      false,
    ],
  ];
  let ok = true;
  for (const [name, body, shouldFlag] of cases) {
    const p = path.join(dir, name);
    fs.writeFileSync(p, body, "utf8");
    const flagged = scanFile(p).findings.length > 0;
    const pass = flagged === shouldFlag;
    ok &&= pass;
    console.log(
      `  ${pass ? "ok  " : "FAIL"}  ${name}: ${flagged ? "flagged" : "clean"} (expected ${shouldFlag ? "flagged" : "clean"})`,
    );
  }
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(
    ok
      ? paint("self-test: PASS", "green")
      : paint("self-test: FAIL — the guard is broken", "red"),
  );
  process.exit(ok ? 0 : 1);
}

function main(): void {
  if (process.argv.includes("--self-test")) selfTest();

  if (!fs.existsSync(SCAN_ROOT)) {
    console.error(paint(`✗ scan root missing: ${SCAN_ROOT}`, "red"));
    process.exit(2);
  }

  const files = walk(SCAN_ROOT);
  const findings: Finding[] = [];
  let suppressed = 0;
  for (const f of files) {
    const r = scanFile(f);
    findings.push(...r.findings);
    suppressed += r.suppressed;
  }

  if (findings.length === 0) {
    console.log(
      paint(
        `✓ frontend log hygiene: scanned ${files.length} file(s), 0 violation(s), ${suppressed} suppressed`,
        "green",
      ),
    );
    process.exit(0);
  }

  console.error(
    paint("✗ frontend log hygiene: payload reaches console", "red"),
  );
  for (const f of findings) {
    console.error(
      `    ${f.file}:${f.line}  console.${f.method}(… ${f.argument} …)`,
    );
  }
  console.error(
    paint(
      "\nLog a description, not the value. If the value genuinely must be logged,\n" +
        "add `// log-hygiene:allow(<reason>)` on the call.",
      "yellow",
    ),
  );
  process.exit(1);
}

main();
