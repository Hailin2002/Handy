#!/usr/bin/env node
// Durable schema validator for Phase-1 spike CSV logs (docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv).
//
// Scope contract (deliberately NOT a reimplementation of analyze-spike-csv.ps1):
// this tool validates SCHEMA ONLY — column contract, metadata presence,
// enum-valid fields, monotonic per-source timestamps, callback-index
// sequencing, and row-completeness against sidecar events.log markers.
// It computes NO statistics (no max gap, drift, histograms, ...); the
// PowerShell analyzer remains the statistical authority.
//
// Usage:
//   node scripts/verify/validate-spike-csv.mjs [<csv> ...] [--events <events.log>] [--tolerance-ms <ms>] [--quiet]
//   node scripts/verify/validate-spike-csv.mjs --self-test
//
// With no <csv> arguments the validator defaults to every committed run CSV
// matching docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv, so CI can
// run it with zero configuration.
//
// Exit codes:
//   0  all validated CSVs pass (informational notices are allowed and do not
//      affect the result; e.g. a run directory without an events.log sidecar,
//      which is git-ignored, only produces a notice)
//   1  validation failure: at least one ERROR finding in any input file
//   2  usage or runtime error (bad arguments, unreadable input file)
//
// Findings format:
//   ERROR   <file>:<line> [CODE] message        -> causes exit 1
//   NOTICE  <file>:<line> [CODE] message        -> informational only
//
// Checks performed per CSV:
//   - '#' metadata lines well-formed; required key `duration_s` present and a
//     positive integer (META_*)
//   - exactly one header row with the exact 16-column contract, in order:
//     source,cb_index,t_rel_ms,gap_ms,frames,total_frames,sample_rate,
//     channels,sample_format,expected_frames,drift_frames,gap_class,silent,
//     peak,rms,error  (HEADER_*)
//   - every data row has exactly 16 fields (COLUMN_COUNT)
//   - enum fields valid: source in {microphone,loopback}; gap_class in
//     {normal,sparse,gap}; silent in {true,false} (ENUM_*)
//   - numeric fields parse (integer: cb_index, frames, total_frames,
//     sample_rate, channels; float: t_rel_ms, gap_ms, expected_frames,
//     drift_frames, peak, rms); sample_rate/channels > 0; sample_format
//     non-empty (NUM_FIELD / EMPTY_FIELD)
//   - per source: t_rel_ms non-decreasing (NON_MONOTONIC_TREL) and cb_index a
//     strictly consecutive 1..N sequence (CB_INDEX_SEQUENCE)
//   - UTF-8 BOM at start of file is tolerated (stripped before parsing)
//   - repeated header row later in the file (DUPLICATE_HEADER)
//   - row-completeness vs events.log (if a sidecar exists next to the CSV or
//     via --events): every timestamped marker line must parse, and every
//     marker's wall-clock offset from the first marker (RUN_LAUNCH) must fall
//     inside the CSV's observed t_rel_ms span +- tolerance (default 2000 ms);
//     markers outside the captured span mean the CSV rows do not cover the
//     scenario timeline (EVENTS_*)

import { readdirSync, readFileSync, existsSync, statSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const EXPECTED_COLUMNS = [
  'source', 'cb_index', 't_rel_ms', 'gap_ms', 'frames', 'total_frames',
  'sample_rate', 'channels', 'sample_format', 'expected_frames',
  'drift_frames', 'gap_class', 'silent', 'peak', 'rms', 'error',
];
const SOURCE_ENUM = new Set(['microphone', 'loopback']);
const GAP_CLASS_ENUM = new Set(['normal', 'sparse', 'gap']);
const SILENT_ENUM = new Set(['true', 'false']);
const REQUIRED_META_KEYS = ['duration_s'];
const INT_COLUMNS = new Set(['cb_index', 'frames', 'total_frames', 'sample_rate', 'channels']);
const FLOAT_COLUMNS = new Set(['t_rel_ms', 'gap_ms', 'expected_frames', 'drift_frames', 'peak', 'rms']);

export function splitCsvLine(line) {
  const fields = [];
  let cur = '';
  let inQ = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (inQ) {
      if (c === '"') {
        if (line[i + 1] === '"') { cur += '"'; i++; } else { inQ = false; }
      } else cur += c;
    } else if (c === '"') inQ = true;
    else if (c === ',') { fields.push(cur); cur = ''; }
    else cur += c;
  }
  fields.push(cur);
  return fields;
}

function parseIntStrict(v) {
  if (!/^[+-]?\d+$/.test(v.trim())) return null;
  const n = Number(v);
  return Number.isSafeInteger(n) ? n : null;
}
function parseFloatStrict(v) {
  if (!/^[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$/.test(v.trim())) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

/**
 * Validate one CSV (as text). Returns { errors: Finding[], notices: Finding[] }
 * where Finding = { line, code, message }.
 */
export function validateCsvText(text, { toleranceMs = 2000, label = '<csv>' } = {}) {
  const errors = [];
  const notices = [];
  const push = (arr, line, code, message) => arr.push({ line, code, message });
  const err = (line, code, message) => push(errors, line, code, message);

  const lines = text.replace(/^\uFEFF/, '').split(/\r?\n/);
  if (lines.length && lines[lines.length - 1] === '') lines.pop();

  const metaKeys = new Map();
  let headerSeenAt = 0;
  let headerFields = null;
  // Per-source state: last t_rel_ms, expected cb_index, min/max t_rel_ms.
  const state = new Map();

  for (let ln = 0; ln < lines.length; ln++) {
    const lineNo = ln + 1;
    const line = lines[ln];

    if (line.startsWith('#')) {
      const body = line.replace(/^#\s?/, '');
      const m = body.match(/^([A-Za-z_]+)[ =](.*)$/);
      if (m) {
        metaKeys.set(m[1], { line: lineNo, value: m[2] });
      } else if (!/^[A-Za-z_]+$/.test(body)) {
        err(lineNo, 'META_BAD_FORMAT', `malformed metadata line: "${body.slice(0, 60)}"`);
      }
      continue;
    }
    if (line.trim() === '') continue;

    const fields = splitCsvLine(line);

    if (!headerSeenAt) {
      // First non-metadata, non-empty line is the header row, whatever its
      // content — a wrong header must be reported as HEADER_MISMATCH, not
      // mistaken for a data row.
      headerSeenAt = lineNo;
      headerFields = fields;
      if (fields.length !== EXPECTED_COLUMNS.length ||
          EXPECTED_COLUMNS.some((c, i) => fields[i] !== c)) {
        const got = fields.join(',');
        err(lineNo, 'HEADER_MISMATCH',
          `header must be exactly: ${EXPECTED_COLUMNS.join(',')} | got (${fields.length} cols): ${got.slice(0, 120)}`);
      }
      continue;
    }
    if (fields[0] === 'source') {
      err(lineNo, 'DUPLICATE_HEADER', 'repeated header row inside data section');
      continue;
    }

    const col = (name) => EXPECTED_COLUMNS.indexOf(name) + 1;
    if (fields.length !== EXPECTED_COLUMNS.length) {
      err(lineNo, 'COLUMN_COUNT',
        `row has ${fields.length} fields, expected ${EXPECTED_COLUMNS.length} (first offending column: ${EXPECTED_COLUMNS[fields.length] ?? '<end>'})`);
      continue;
    }

    const get = (name) => fields[col(name) - 1];
    const source = get('source');
    if (!SOURCE_ENUM.has(source)) {
      err(lineNo, 'ENUM_SOURCE', `column ${col('source')} (source): invalid value "${source}", expected one of ${[...SOURCE_ENUM].join('|')}`);
    }
    const gapClass = get('gap_class');
    if (!GAP_CLASS_ENUM.has(gapClass)) {
      err(lineNo, 'ENUM_GAP_CLASS', `column ${col('gap_class')} (gap_class): invalid value "${gapClass}", expected one of ${[...GAP_CLASS_ENUM].join('|')}`);
    }
    const silent = get('silent');
    if (!SILENT_ENUM.has(silent)) {
      err(lineNo, 'ENUM_SILENT', `column ${col('silent')} (silent): invalid value "${silent}", expected true|false`);
    }
    if (get('sample_format').trim() === '') {
      err(lineNo, 'EMPTY_FIELD', `column ${col('sample_format')} (sample_format) must be non-empty`);
    }

    for (const name of INT_COLUMNS) {
      const v = get(name);
      const n = parseIntStrict(v);
      if (n === null) err(lineNo, 'NUM_FIELD', `column ${col(name)} (${name}): "${v}" is not a valid integer`);
    }
    for (const name of FLOAT_COLUMNS) {
      const v = get(name);
      const n = parseFloatStrict(v);
      if (n === null) err(lineNo, 'NUM_FIELD', `column ${col(name)} (${name}): "${v}" is not a valid number`);
    }
    const rate = parseFloatStrict(get('sample_rate'));
    if (rate !== null && !(rate > 0)) err(lineNo, 'NUM_FIELD', `column ${col('sample_rate')} (sample_rate) must be > 0, got ${get('sample_rate')}`);
    const ch = parseIntStrict(get('channels'));
    if (ch !== null && !(ch > 0)) err(lineNo, 'NUM_FIELD', `column ${col('channels')} (channels) must be > 0, got ${get('channels')}`);

    if (SOURCE_ENUM.has(source)) {
      let st = state.get(source);
      if (!st) { st = { lastT: null, nextCb: 1, minT: Infinity, maxT: -Infinity }; state.set(source, st); }
      const t = parseFloatStrict(get('t_rel_ms'));
      const cb = parseIntStrict(get('cb_index'));
      if (t !== null) {
        if (st.lastT !== null && t < st.lastT) {
          err(lineNo, 'NON_MONOTONIC_TREL', `source "${source}": t_rel_ms went backwards: ${t} < previous ${st.lastT}`);
        }
        st.lastT = Math.max(st.lastT ?? -Infinity, t);
        st.minT = Math.min(st.minT, t);
        st.maxT = Math.max(st.maxT, t);
      }
      if (cb !== null) {
        if (cb !== st.nextCb) {
          err(lineNo, 'CB_INDEX_SEQUENCE', `source "${source}": cb_index is ${cb}, expected ${st.nextCb} (must be consecutive from 1)`);
          st.nextCb = cb + 1;
        } else st.nextCb++;
      }
    }
  }

  if (!headerSeenAt) err(0, 'HEADER_MISSING', 'no "source,..." header row found');

  for (const key of REQUIRED_META_KEYS) {
    const entry = metaKeys.get(key);
    if (!entry) { err(0, 'META_MISSING_KEY', `required metadata "# ${key}=" line not found`); continue; }
    const n = parseIntStrict(entry.value);
    if (n === null || n <= 0) {
      err(entry.line, 'META_BAD_VALUE', `metadata ${key}=${entry.value} must be a positive integer`);
    }
  }
  if (!metaKeys.has('streams_started')) {
    push(notices, 0, 'META_NOTICE', 'optional "# streams_started" metadata absent');
  }
  if (!metaKeys.has('run_finished')) {
    push(notices, 0, 'META_NOTICE', '"# run_finished" absent: capture did not complete (aborted run)');
  }

  return { errors, notices, span: collectSpan(state), metaKeys };
}

function collectSpan(state) {
  let minT = Infinity; let maxT = -Infinity;
  for (const st of state.values()) {
    if (st.minT < minT) minT = st.minT;
    if (st.maxT > maxT) maxT = st.maxT;
  }
  return Number.isFinite(minT) ? { minT, maxT } : null;
}

const EVENT_TS = /^(\d{4}-\d{2}-\d{2}) (\d{2}):(\d{2}):(\d{2})\.(\d{3})([+-]\d{2}):?(\d{2}) (.*)$/;

/** Parse events.log text into [{ line, offsetMs, message }] sorted by offset. */
export function parseEventsLog(text) {
  const out = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim() === '') continue;
    const m = line.match(EVENT_TS);
    if (!m) { out.push({ line: i + 1, offsetMs: null, message: line }); continue; }
    const [, date, hh, mm, ss, ms, osign, omin, msg] = m;
    const utc = Date.parse(`${date}T${hh}:${mm}:${ss}.${ms}Z`);
    const local = Date.parse(`${date}T${hh}:${mm}:${ss}.${ms}${osign}:${omin}`);
    const t = Number.isFinite(local) ? local : utc;
    out.push({ line: i + 1, offsetMs: Number.isFinite(t) ? t : null, message: msg });
  }
  const base = out.find((e) => e.offsetMs !== null);
  for (const e of out) {
    if (e.offsetMs !== null && base) e.relMs = e.offsetMs - base.offsetMs;
  }
  return out;
}

/**
 * Cross-check CSV row coverage against events.log markers.
 * Returns findings { errors, notices }.
 */
export function checkEventCoverage(csvSpan, events, { toleranceMs = 2000, label = '<csv>' } = {}) {
  const errors = [];
  const notices = [];
  if (!csvSpan) return { errors, notices };
  for (const ev of events) {
    if (ev.offsetMs === null || ev.relMs === undefined) {
      errors.push({ line: ev.line, code: 'EVENTS_BAD_LINE', message: `events.log line does not start with "yyyy-MM-dd HH:mm:ss.fffzzz": "${ev.message.slice(0, 60)}"` });
      continue;
    }
    if (ev.relMs > csvSpan.maxT + toleranceMs) {
      errors.push({
        line: ev.line,
        code: 'EVENTS_MARKER_UNCOVERED',
        message: `marker "${ev.message.slice(0, 50)}" at +${Math.round(ev.relMs)}ms lies beyond CSV coverage (last t_rel_ms=${csvSpan.maxT.toFixed(3)}, tolerance=${toleranceMs}ms): CSV rows do not cover the scenario timeline`,
      });
    }
  }
  if (events.length) {
    const named = events.filter((e) => e.message.startsWith('RUN_FINISHED')).length;
    if (!named) notices.push({ line: 0, code: 'EVENTS_NOTICE', message: 'no RUN_FINISHED marker in events.log' });
  }
  return { errors, notices };
}

function discoverDefaultCsvs(root) {
  const dir = join(root, 'docs', 'phase1-spike');
  if (!existsSync(dir)) return [];
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (!statSync(p).isDirectory()) continue;
    const csv = join(p, 'meeting_loopback_spike_120min.csv');
    if (existsSync(csv)) out.push(csv);
  }
  return out.sort();
}

function findEventsSidecar(csvPath) {
  const sibling = join(dirname(csvPath), 'events.log');
  return existsSync(sibling) ? sibling : null;
}

function fmt(file, f) {
  const where = f.line ? `${file}:${f.line}` : file;
  return `${where} [${f.code}] ${f.message}`;
}

export function validateFile(csvPath, { eventsPath, toleranceMs, quiet }) {
  const text = readFileSync(csvPath, 'utf8');
  const res = validateCsvText(text, { toleranceMs });
  const errors = [...res.errors];
  const notices = [...res.notices];

  let eventsFile = eventsPath ?? findEventsSidecar(csvPath);
  if (eventsPath && !existsSync(eventsPath)) {
    errors.push({ line: 0, code: 'EVENTS_NOT_FOUND', message: `--events file not found: ${eventsPath}` });
    eventsFile = null;
  }
  if (eventsFile) {
    const ev = parseEventsLog(readFileSync(eventsFile, 'utf8'));
    const cov = checkEventCoverage(res.span, ev, { toleranceMs });
    errors.push(...cov.errors);
    notices.push(...cov.notices);
  } else {
    notices.push({ line: 0, code: 'EVENTS_NOTICE', message: 'no events.log sidecar found (git-ignored); skipping row-completeness cross-check' });
  }

  for (const n of notices) if (!quiet) console.log(`NOTICE  ${fmt(csvPath, n)}`);
  for (const e of errors) console.error(`ERROR   ${fmt(csvPath, e)}`);
  if (!errors.length && !quiet) console.log(`OK      ${csvPath} (${res.span ? `rows span t_rel_ms ${res.span.minT}..${Math.round(res.span.maxT)}` : 'no data rows'})`);
  return errors.length === 0;
}

function selfTest(toleranceMs) {
  const goodMeta = [
    '# meeting_loopback_spike run',
    '# duration_s=7200',
    '# streams_started',
  ].join('\n');
  const goodHeader = EXPECTED_COLUMNS.join(',');
  const row = (src, cb, t, gap, cls = 'normal', silent = 'false', errCol = '') =>
    `${src},${cb},${t},${gap},480,480,48000,2,f32,551.0,-71.0,${cls},${silent},0.001,0.001,${errCol}`;
  const goodCsv = [goodMeta, goodHeader, row('microphone', 1, 10, 10), row('microphone', 2, 20.5, 10.5), row('loopback', 1, 12, 12)].join('\n') + '\n';

  const cases = [];
  const caseDef = (name, csvText, expectCode, extra) => ({ name, csvText, expectCode, ...extra });

  cases.push(caseDef('valid-baseline', goodCsv, null));
  cases.push(caseDef('header-wrong-column-order', goodCsv.replace(goodHeader, EXPECTED_COLUMNS.slice().reverse().join(',')), 'HEADER_MISMATCH'));
  cases.push(caseDef('row-too-few-columns', goodCsv + 'microphone,3,30\n', 'COLUMN_COUNT'));
  cases.push(caseDef('enum-invalid-gap-class', goodCsv + row('microphone', 3, 30, 10, 'huge'), 'ENUM_GAP_CLASS'));
  cases.push(caseDef('enum-invalid-source', goodCsv + row('aux_input', 1, 30, 10), 'ENUM_SOURCE'));
  cases.push(caseDef('enum-invalid-silent', goodCsv + row('microphone', 3, 30, 10, 'normal', 'maybe'), 'ENUM_SILENT'));
  cases.push(caseDef('numeric-garbage', goodCsv + row('microphone', 3, 'NaN', 10), 'NUM_FIELD'));
  cases.push(caseDef('non-monotonic-timestamp', goodCsv + row('microphone', 3, 5, 10), 'NON_MONOTONIC_TREL'));
  cases.push(caseDef('cb-index-skipped', goodCsv + row('microphone', 4, 30, 10), 'CB_INDEX_SEQUENCE'));
  cases.push(caseDef('missing-duration-meta', ['# run', goodHeader, row('microphone', 1, 10, 10)].join('\n'), 'META_MISSING_KEY'));
  cases.push(caseDef('duplicate-header', goodCsv + goodHeader + '\n', 'DUPLICATE_HEADER'));

  let failures = 0;
  for (const c of cases) {
    const r = validateCsvText(c.csvText, { toleranceMs });
    const codes = r.errors.map((e) => e.code);
    if (c.expectCode === null) {
      if (r.errors.length) { console.error(`SELFTEST FAIL ${c.name}: expected clean pass, got ${codes.join(',')}`); failures++; }
      else console.log(`selftest ok ${c.name}`);
    } else if (!codes.includes(c.expectCode)) {
      console.error(`SELFTEST FAIL ${c.name}: expected ${c.expectCode}, got ${codes.join(',') || '(none)'}`);
      failures++;
    } else {
      const e = r.errors.find((x) => x.code === c.expectCode);
      console.log(`selftest ok ${c.name} -> [${e.code}] ${e.message.slice(0, 100)}`);
    }
  }

  // Event-coverage cross-check: marker beyond CSV span must be rejected.
  const eventsText = [
    '2026-08-22 12:00:00.000+02:00 RUN_LAUNCH exe=spike.exe',
    '2026-08-22 13:01:00.000+02:00 TONE_START idx=1',
  ].join('\n');
  const ev = parseEventsLog(eventsText);
  const covOk = checkEventCoverage({ minT: 0, maxT: 4000000 }, ev, { toleranceMs });
  const covBad = checkEventCoverage({ minT: 0, maxT: 300000 }, ev, { toleranceMs });
  if (covOk.errors.length !== 0) { console.error(`SELFTEST FAIL events-covered: expected no errors, got ${JSON.stringify(covOk.errors)}`); failures++; }
  else console.log('selftest ok events-covered');
  if (!covBad.errors.some((e) => e.code === 'EVENTS_MARKER_UNCOVERED')) { console.error('SELFTEST FAIL events-uncovered: expected EVENTS_MARKER_UNCOVERED'); failures++; }
  else console.log(`selftest ok events-uncovered -> ${covBad.errors[0].message.slice(0, 90)}`);
  const badEv = parseEventsLog('garbage line without timestamp');
  if (!badEv.some((e) => e.offsetMs === null)) { console.error('SELFTEST FAIL events-parse: unparseable line not flagged'); failures++; }
  else console.log('selftest ok events-parse');

  console.log(failures === 0 ? `SELF-TEST PASS (${cases.length + 3} checks)` : `SELF-TEST FAIL (${failures} failing checks)`);
  return failures === 0 ? 0 : 1;
}

function main(argv) {
  const args = argv.slice(2);
  if (args.includes('--help') || args.includes('-h')) {
    console.log('usage: node validate-spike-csv.mjs [<csv>...] [--events <file>] [--tolerance-ms <ms>] [--quiet] | --self-test');
    console.log('exit codes: 0 = all valid, 1 = validation failure, 2 = usage/runtime error');
    return 0;
  }
  const toleranceIdx = args.indexOf('--tolerance-ms');
  let toleranceMs = 2000;
  if (toleranceIdx !== -1) {
    toleranceMs = parseIntStrict(args[toleranceIdx + 1] ?? '');
    if (toleranceMs === null || toleranceMs < 0) {
      console.error('error: --tolerance-ms requires a non-negative integer');
      return 2;
    }
    args.splice(toleranceIdx, 2);
  }
  const quiet = args.includes('--quiet');
  const eventsIdx = args.indexOf('--events');
  let eventsPath = null;
  if (eventsIdx !== -1) {
    eventsPath = args[eventsIdx + 1];
    if (!eventsPath) { console.error('error: --events requires a path'); return 2; }
    args.splice(eventsIdx, 2);
  }
  if (args.includes('--self-test')) return selfTest(toleranceMs);

  // Reject unrecognized flags with exit 2 so CI never silently ignores a
  // typo like "--toleranc-ms 500" (whose value would otherwise be misread
  // as a CSV path).
  const KNOWN_FLAGS = new Set(['--self-test', '--quiet']);
  const unknown = args.filter((a) => a.startsWith('--') && !KNOWN_FLAGS.has(a));
  if (unknown.length) {
    console.error(`error: unknown flag(s): ${unknown.join(' ')} (known: --events, --tolerance-ms, --quiet, --self-test)`);
    return 2;
  }

  let csvs = args.filter((a) => !a.startsWith('--'));
  if (!csvs.length) {
    const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
    csvs = discoverDefaultCsvs(root);
    if (!csvs.length) { console.error('error: no CSV arguments and no default run CSVs under docs/phase1-spike'); return 2; }
    if (!quiet) console.log(`validating ${csvs.length} committed run CSV(s) (default discovery)`);
  }

  let ok = true;
  for (const csv of csvs) {
    try {
      if (!validateFile(resolve(csv), { eventsPath, toleranceMs, quiet })) ok = false;
    } catch (e) {
      console.error(`ERROR   ${resolve(csv)} [READ_FAILED] ${e.message}`);
      ok = false;
    }
  }
  return ok ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main(process.argv));
}
