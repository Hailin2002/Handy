import fs from "node:fs";
import process from "node:process";
import * as SpeechSDK from "microsoft-cognitiveservices-speech-sdk";

const wavPath = process.argv[2];
const chunkMs = Number(process.argv[3] ?? "30");
const segmentationSilenceMs = Number(process.argv[4] ?? "300");
const stablePartialThreshold = Number(process.argv[5] ?? "1");
const maxAudioMs = Number(process.argv[6] ?? "0");
const region = process.env.SPEAK_AZURE_REGION;
const key = process.env.SPEAK_AZURE_KEY;
const language = process.env.SPEAK_AZURE_LANGUAGE ?? "de-DE";

if (!wavPath || !Number.isInteger(chunkMs) || chunkMs <= 0 || !region || !key) {
  console.error(
    "usage: SPEAK_AZURE_REGION=... SPEAK_AZURE_KEY=... node azure-stream-cadence-replay.mjs <wav> <chunk-ms>",
  );
  process.exit(2);
}

const wav = fs.readFileSync(wavPath);
if (
  wav.toString("ascii", 0, 4) !== "RIFF" ||
  wav.toString("ascii", 8, 12) !== "WAVE"
) {
  throw new Error("Only RIFF/WAVE input is supported");
}

let offset = 12;
let format;
let pcm;
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
    pcm = wav.subarray(start, start + size);
  }
  offset = start + size + (size % 2);
}

if (
  !format ||
  !pcm ||
  format.encoding !== 1 ||
  format.channels !== 1 ||
  format.sampleRate !== 16_000 ||
  format.bits !== 16
) {
  throw new Error(
    `Expected mono 16 kHz PCM16 WAV; got ${JSON.stringify(format)}`,
  );
}

// Match the realtime gain used by the Rust recording path. Processing remains
// in native 30 ms recorder frames even when the transport batches 120 ms.
if (maxAudioMs > 0) {
  pcm = pcm.subarray(
    0,
    Math.min(pcm.length, Math.round((16_000 * 2 * maxAudioMs) / 1_000)),
  );
}
const gained = Buffer.allocUnsafe(pcm.length);
let currentGain = 8;
let minGain = 8;
let maxGain = 8;
let gainSum = 0;
let gainFrames = 0;
const recorderFrameBytes = (16_000 * 2 * 30) / 1_000;
for (
  let frameStart = 0;
  frameStart < pcm.length;
  frameStart += recorderFrameBytes
) {
  const frameEnd = Math.min(frameStart + recorderFrameBytes, pcm.length);
  let peak = 0;
  for (let index = frameStart; index + 1 < frameEnd; index += 2) {
    peak = Math.max(peak, Math.abs(pcm.readInt16LE(index) / 32768));
  }
  const desired = peak === 0 ? 8 : Math.max(1, Math.min(8, 0.75 / peak));
  currentGain =
    desired < currentGain
      ? desired
      : currentGain + (desired - currentGain) * 0.15;
  minGain = Math.min(minGain, currentGain);
  maxGain = Math.max(maxGain, currentGain);
  gainSum += currentGain;
  gainFrames += 1;
  for (let index = frameStart; index + 1 < frameEnd; index += 2) {
    const sample = pcm.readInt16LE(index) / 32768;
    const output = Math.trunc(
      Math.max(-1, Math.min(1, sample * currentGain)) * 32767,
    );
    gained.writeInt16LE(output, index);
  }
}

const speechConfig = SpeechSDK.SpeechConfig.fromSubscription(key, region);
speechConfig.speechRecognitionLanguage = language;
speechConfig.setProperty(
  SpeechSDK.PropertyId.SpeechServiceResponse_StablePartialResultThreshold,
  String(stablePartialThreshold),
);
speechConfig.setProperty(
  SpeechSDK.PropertyId.Speech_SegmentationSilenceTimeoutMs,
  String(segmentationSilenceMs),
);
const streamFormat = SpeechSDK.AudioStreamFormat.getWaveFormatPCM(
  16_000,
  16,
  1,
);
const pushStream = SpeechSDK.AudioInputStream.createPushStream(streamFormat);
const audioConfig = SpeechSDK.AudioConfig.fromStreamInput(pushStream);
const recognizer = new SpeechSDK.SpeechRecognizer(speechConfig, audioConfig);

const segments = [];
let latestInterim = "";
let recognizingEvents = 0;
let firstEventMs;
let lastEventMs;
let recognizedAudioEndMs = 0;
let canceledError;
const startedAt = performance.now();

recognizer.recognizing = (_sender, event) => {
  const text = event.result.text?.trim();
  if (!text) return;
  recognizingEvents += 1;
  latestInterim = text;
  recognizedAudioEndMs = Math.max(
    recognizedAudioEndMs,
    Math.round((event.result.offset + event.result.duration) / 10_000),
  );
  firstEventMs ??= performance.now() - startedAt;
  lastEventMs = performance.now() - startedAt;
};
recognizer.recognized = (_sender, event) => {
  const text = event.result.text?.trim();
  if (event.result.reason === SpeechSDK.ResultReason.RecognizedSpeech && text) {
    segments.push(text);
    latestInterim = "";
    recognizedAudioEndMs = Math.max(
      recognizedAudioEndMs,
      Math.round((event.result.offset + event.result.duration) / 10_000),
    );
    firstEventMs ??= performance.now() - startedAt;
    lastEventMs = performance.now() - startedAt;
  }
};
recognizer.canceled = (_sender, event) => {
  if (event.reason === SpeechSDK.CancellationReason.Error) {
    canceledError = event.errorDetails || String(event.reason);
  }
};

await new Promise((resolve, reject) =>
  recognizer.startContinuousRecognitionAsync(resolve, reject),
);

const chunkBytes = Math.round((16_000 * 2 * chunkMs) / 1_000);
let chunks = 0;
for (let start = 0; start < gained.length; start += chunkBytes) {
  const chunk = gained.subarray(
    start,
    Math.min(start + chunkBytes, gained.length),
  );
  pushStream.write(chunk);
  chunks += 1;
  await new Promise((resolve) =>
    setTimeout(resolve, (chunk.length / 2 / 16_000) * 1_000),
  );
}
pushStream.write(new ArrayBuffer((16_000 * 2 * 300) / 1_000));
pushStream.close();
const stopStartedAt = performance.now();
await new Promise((resolve, reject) =>
  recognizer.stopContinuousRecognitionAsync(resolve, reject),
);
const stopMs = performance.now() - stopStartedAt;
recognizer.close();
try {
  audioConfig.close();
} catch {
  // The Node SDK already tears down the stream with the recognizer on some
  // versions and then throws from a redundant AudioConfig.close().
}

const complete = [...segments, latestInterim]
  .map((segment) => segment.trim())
  .filter(Boolean)
  .join(" ");
console.log(
  JSON.stringify({
    file: wavPath.split(/[\\/]/).at(-1),
    audioSeconds: pcm.length / 2 / 16_000,
    chunkMs,
    segmentationSilenceMs,
    stablePartialThreshold,
    maxAudioMs: maxAudioMs || null,
    chunks,
    bytes: gained.length,
    finalSegments: segments.length,
    chars: [...complete].length,
    words: complete.split(/\s+/u).filter(Boolean).length,
    recognizingEvents,
    firstEventMs: Math.round(firstEventMs ?? -1),
    lastEventMs: Math.round(lastEventMs ?? -1),
    recognizedAudioEndMs,
    stopMs: Math.round(stopMs),
    gain: {
      min: Number(minGain.toFixed(2)),
      average: Number((gainSum / Math.max(1, gainFrames)).toFixed(2)),
      max: Number(maxGain.toFixed(2)),
    },
    error: canceledError ?? null,
  }),
);
