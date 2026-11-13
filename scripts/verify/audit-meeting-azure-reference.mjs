import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";

const meetingId = process.argv[2];
const outputPath = process.argv[3];
const chunkFilter = process.argv[4] ?? "";
const appData = process.env.APPDATA;
const endpoint = process.env.SPEAK_AZURE_ENDPOINT;
const key = process.env.SPEAK_AZURE_KEY;
const language = process.env.SPEAK_AZURE_LANGUAGE ?? "de-DE";
const concurrency = Math.max(
  1,
  Math.min(8, Number(process.env.SPEAK_AZURE_AUDIT_CONCURRENCY ?? "4")),
);

if (!meetingId || !outputPath || !appData || !endpoint || !key) {
  console.error(
    "usage: audit-meeting-azure-reference.mjs <meeting-id> <output-json>",
  );
  process.exit(2);
}

const dataRoot = path.join(appData, "com.haeni.speak");
const meetingRoot = path.join(dataRoot, "meetings", meetingId);
const database = new DatabaseSync(path.join(dataRoot, "meetings.db"), {
  readOnly: true,
});

const session = database
  .prepare(
    `SELECT id, title, state, stt_provider, language, started_at, ended_at
       FROM meeting_sessions
      WHERE id = ? AND deleted_at IS NULL`,
  )
  .get(meetingId);
if (!session) throw new Error(`Meeting ${meetingId} was not found`);

let chunks = database
  .prepare(
    `SELECT id, sequence, source, timeline_start_ms, timeline_end_ms,
            storage_key, sample_rate, sample_count, file_size, checksum, state
       FROM meeting_audio_chunks
      WHERE meeting_id = ? AND deleted_at IS NULL
      ORDER BY timeline_start_ms, source, sequence`,
  )
  .all(meetingId);

if (chunkFilter) {
  const selected = new Set(chunkFilter.split(",").map((item) => item.trim()));
  chunks = chunks.filter((chunk) =>
    selected.has(`${chunk.source}:${chunk.sequence}`),
  );
}

const localSegmentsForChunk = database.prepare(
  `SELECT t.text
     FROM meeting_stt_job_chunks jc
     JOIN meeting_stt_jobs j ON j.id = jc.job_id
     JOIN meeting_transcript_segments t ON t.job_id = j.id
    WHERE jc.chunk_id = ? AND t.deleted_at IS NULL
    ORDER BY t.sequence, t.start_ms`,
);

const tokenize = (text) =>
  text
    .normalize("NFKC")
    .toLocaleLowerCase("de-DE")
    .match(/[\p{L}\p{N}]+/gu) ?? [];

const editDistance = (left, right) => {
  let previous = Array.from({ length: right.length + 1 }, (_, index) => index);
  for (let leftIndex = 1; leftIndex <= left.length; leftIndex += 1) {
    const current = [leftIndex];
    for (let rightIndex = 1; rightIndex <= right.length; rightIndex += 1) {
      current[rightIndex] = Math.min(
        previous[rightIndex] + 1,
        current[rightIndex - 1] + 1,
        previous[rightIndex - 1] +
          (left[leftIndex - 1] === right[rightIndex - 1] ? 0 : 1),
      );
    }
    previous = current;
  }
  return previous[right.length];
};

const bagOverlap = (localWords, azureWords) => {
  const counts = new Map();
  for (const word of azureWords) counts.set(word, (counts.get(word) ?? 0) + 1);
  let overlap = 0;
  for (const word of localWords) {
    const remaining = counts.get(word) ?? 0;
    if (remaining > 0) {
      overlap += 1;
      counts.set(word, remaining - 1);
    }
  }
  return overlap;
};

const parsePcm16Wav = (wav) => {
  if (
    wav.toString("ascii", 0, 4) !== "RIFF" ||
    wav.toString("ascii", 8, 12) !== "WAVE"
  ) {
    throw new Error("not a RIFF/WAVE file");
  }
  let offset = 12;
  let format;
  let dataStart;
  let dataSize;
  while (offset + 8 <= wav.length) {
    const id = wav.toString("ascii", offset, offset + 4);
    const size = wav.readUInt32LE(offset + 4);
    const start = offset + 8;
    if (id === "fmt ") {
      format = {
        encoding: wav.readUInt16LE(start),
        channels: wav.readUInt16LE(start + 2),
        sampleRate: wav.readUInt32LE(start + 4),
        bits: wav.readUInt16LE(start + 14),
      };
    } else if (id === "data") {
      dataStart = start;
      dataSize = size;
      break;
    }
    offset = start + size + (size % 2);
  }
  if (
    !format ||
    dataStart == null ||
    format.encoding !== 1 ||
    format.channels !== 1 ||
    format.sampleRate !== 16_000 ||
    format.bits !== 16
  ) {
    throw new Error(`unsupported WAV format ${JSON.stringify(format)}`);
  }
  return { dataStart, dataSize };
};

const prepareAzureWav = (original) => {
  const { dataStart, dataSize } = parsePcm16Wav(original);
  let peak = 0;
  let squareSum = 0;
  let sampleCount = 0;
  for (let index = dataStart; index + 1 < dataStart + dataSize; index += 2) {
    const sample = original.readInt16LE(index) / 32768;
    peak = Math.max(peak, Math.abs(sample));
    squareSum += sample * sample;
    sampleCount += 1;
  }
  const rms = Math.sqrt(squareSum / Math.max(1, sampleCount));
  const gain =
    peak <= Number.EPSILON ? 1 : Math.max(1, Math.min(8, 0.75 / peak));
  if (gain <= 1.01) return { wav: original, peak, rms, gain: 1 };

  const wav = Buffer.from(original);
  for (let index = dataStart; index + 1 < dataStart + dataSize; index += 2) {
    const amplified = Math.trunc(
      Math.max(-1, Math.min(1, (original.readInt16LE(index) / 32768) * gain)) *
        32767,
    );
    wav.writeInt16LE(amplified, index);
  }
  return { wav, peak, rms, gain };
};

const wait = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

const transcribeAzure = async (wav, fileName) => {
  let lastError;
  for (let attempt = 1; attempt <= 5; attempt += 1) {
    const form = new FormData();
    form.append("definition", JSON.stringify({ locales: [language] }));
    form.append("audio", new Blob([wav], { type: "audio/wav" }), fileName);
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 120_000);
    const startedAt = performance.now();
    try {
      const response = await fetch(
        `${endpoint}/speechtotext/transcriptions:transcribe?api-version=2025-10-15`,
        {
          method: "POST",
          headers: { "Ocp-Apim-Subscription-Key": key },
          body: form,
          signal: controller.signal,
        },
      );
      const elapsedMs = Math.round(performance.now() - startedAt);
      const body = await response.json().catch(() => ({}));
      if (response.ok) return { body, elapsedMs, attempts: attempt };
      lastError = `HTTP ${response.status}: ${body.error?.code ?? "request-failed"}`;
      if (response.status !== 429 && response.status < 500) break;
      const retryAfter = Number(response.headers.get("retry-after"));
      await wait(
        Number.isFinite(retryAfter) ? retryAfter * 1_000 : 750 * 2 ** attempt,
      );
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
      await wait(750 * 2 ** attempt);
    } finally {
      clearTimeout(timeout);
    }
  }
  throw new Error(lastError ?? "Azure request failed");
};

const auditChunk = async (chunk) => {
  const filePath = path.join(meetingRoot, chunk.storage_key);
  const original = fs.readFileSync(filePath);
  const { wav, peak, rms, gain } = prepareAzureWav(original);
  const localText = localSegmentsForChunk
    .all(chunk.id)
    .map((segment) => segment.text.trim())
    .filter(Boolean)
    .join(" ");
  const localWords = tokenize(localText);
  const { body, elapsedMs, attempts } = await transcribeAzure(
    wav,
    path.basename(filePath),
  );
  const azureText = (body.combinedPhrases ?? [])
    .map((phrase) => phrase.text?.trim() ?? "")
    .filter(Boolean)
    .join(" ");
  const azureWords = tokenize(azureText);
  const overlap = bagOverlap(localWords, azureWords);
  const wordDistance = editDistance(localWords, azureWords);
  const wordRatio = localWords.length / Math.max(1, azureWords.length);
  const recall = overlap / Math.max(1, azureWords.length);
  const precision = overlap / Math.max(1, localWords.length);
  const wordErrorRate = wordDistance / Math.max(1, azureWords.length);
  const coverageEndMs = Math.max(
    0,
    ...(body.phrases ?? []).map(
      (phrase) =>
        Number(phrase.offsetMilliseconds ?? 0) +
        Number(phrase.durationMilliseconds ?? 0),
    ),
  );

  let classification = "different";
  if (localWords.length === 0 && azureWords.length === 0)
    classification = "both_empty";
  else if (localWords.length < 3 && azureWords.length >= 5)
    classification = "local_missing";
  else if (azureWords.length < 3 && localWords.length >= 5)
    classification = "azure_missing";
  else if (wordRatio < 0.65) classification = "local_much_shorter";
  else if (wordRatio > 1.55) classification = "local_much_longer";
  else if (wordErrorRate <= 0.35 && recall >= 0.7) classification = "close";

  return {
    sequence: Number(chunk.sequence),
    source: chunk.source,
    timelineStartMs: Number(chunk.timeline_start_ms),
    timelineEndMs: Number(chunk.timeline_end_ms),
    audioDurationMs: Math.round((Number(chunk.sample_count) / 16_000) * 1_000),
    audioPeak: Number(peak.toFixed(5)),
    audioRms: Number(rms.toFixed(5)),
    azureGain: Number(gain.toFixed(2)),
    localWords: localWords.length,
    azureWords: azureWords.length,
    wordRatio: Number(wordRatio.toFixed(3)),
    tokenRecall: Number(recall.toFixed(3)),
    tokenPrecision: Number(precision.toFixed(3)),
    wordErrorRate: Number(wordErrorRate.toFixed(3)),
    azurePhrases: body.phrases?.length ?? 0,
    azureCoverageEndMs: coverageEndMs,
    azureElapsedMs: elapsedMs,
    azureAttempts: attempts,
    classification,
    ...(process.env.SPEAK_AZURE_AUDIT_INCLUDE_TEXT === "1"
      ? { localText, azureText }
      : {}),
  };
};

const results = new Array(chunks.length);
let nextIndex = 0;
let completed = 0;
const failures = [];
const startedAt = performance.now();

const worker = async () => {
  while (true) {
    const index = nextIndex;
    nextIndex += 1;
    if (index >= chunks.length) return;
    const chunk = chunks[index];
    try {
      results[index] = await auditChunk(chunk);
    } catch (error) {
      const failure = {
        sequence: Number(chunk.sequence),
        source: chunk.source,
        error: error instanceof Error ? error.message : String(error),
      };
      results[index] = { ...failure, classification: "audit_error" };
      failures.push(failure);
    }
    completed += 1;
    if (completed % 10 === 0 || completed === chunks.length) {
      console.log(`Azure audit progress ${completed}/${chunks.length}`);
    }
  }
};

await Promise.all(Array.from({ length: concurrency }, () => worker()));
database.close();

const successful = results.filter(
  (result) => result.classification !== "audit_error",
);
const summarize = (subset) => {
  const classifications = {};
  for (const result of subset) {
    classifications[result.classification] =
      (classifications[result.classification] ?? 0) + 1;
  }
  return {
    chunks: subset.length,
    localWords: subset.reduce((sum, result) => sum + result.localWords, 0),
    azureWords: subset.reduce((sum, result) => sum + result.azureWords, 0),
    classifications,
    medianAzureMs: [...subset].sort(
      (left, right) => left.azureElapsedMs - right.azureElapsedMs,
    )[Math.floor(subset.length / 2)]?.azureElapsedMs,
  };
};

const report = {
  schemaVersion: 1,
  generatedAt: new Date().toISOString(),
  meeting: {
    id: session.id,
    title: session.title,
    state: session.state,
    originalProvider: session.stt_provider,
    language: session.language,
    startedAt: Number(session.started_at),
    endedAt: Number(session.ended_at),
  },
  azureReference: {
    apiVersion: "2025-10-15",
    language,
    concurrency,
  },
  elapsedMs: Math.round(performance.now() - startedAt),
  summary: summarize(successful),
  bySource: Object.fromEntries(
    ["microphone", "system"].map((source) => [
      source,
      summarize(successful.filter((result) => result.source === source)),
    ]),
  ),
  failures,
  results,
};

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
console.log(
  JSON.stringify({
    outputPath,
    elapsedMs: report.elapsedMs,
    summary: report.summary,
    bySource: report.bySource,
    failures: failures.length,
  }),
);
