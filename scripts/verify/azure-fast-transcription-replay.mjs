import fs from "node:fs";
import process from "node:process";

const wavPath = process.argv[2];
const endpoint = process.env.SPEAK_AZURE_ENDPOINT;
const key = process.env.SPEAK_AZURE_KEY;
const language = process.env.SPEAK_AZURE_LANGUAGE ?? "de-DE";
if (!wavPath || !endpoint || !key) {
  console.error("Azure endpoint, key, and WAV path are required");
  process.exit(2);
}

const audio = fs.readFileSync(wavPath);
const form = new FormData();
form.append("definition", JSON.stringify({ locales: [language] }));
form.append("audio", new Blob([audio], { type: "audio/wav" }), "recording.wav");

const startedAt = performance.now();
const response = await fetch(
  `${endpoint}/speechtotext/transcriptions:transcribe?api-version=2025-10-15`,
  {
    method: "POST",
    headers: { "Ocp-Apim-Subscription-Key": key },
    body: form,
  },
);
const elapsedMs = performance.now() - startedAt;
const body = await response.json().catch(() => ({}));
if (!response.ok) {
  console.error(
    JSON.stringify({
      status: response.status,
      elapsedMs: Math.round(elapsedMs),
      error: body.error?.code ?? "request-failed",
    }),
  );
  process.exit(1);
}
const complete = (body.combinedPhrases ?? [])
  .map((phrase) => phrase.text?.trim() ?? "")
  .filter(Boolean)
  .join(" ");
console.log(
  JSON.stringify({
    file: wavPath.split(/[\\/]/).at(-1),
    elapsedMs: Math.round(elapsedMs),
    durationMs: body.durationMilliseconds ?? null,
    phrases: body.phrases?.length ?? 0,
    chars: [...complete].length,
    words: complete.split(/\s+/u).filter(Boolean).length,
  }),
);
