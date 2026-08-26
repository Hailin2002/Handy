import React from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { MeetingAudioFileDto } from "@/bindings";
import { formatMeetingDuration } from "@/components/shell/productNav";

interface MeetingAudioPanelProps {
  files: MeetingAudioFileDto[];
  /** Seek target in timeline milliseconds, set by a transcript jump. */
  seekToMs: number | null;
}

/**
 * `Audio` tab (plan §6.13): playback of the stored chunks, per source and in
 * timeline order, "nur solange Audio vorhanden ist".
 *
 * Chunks are separate files rather than one stream, so the panel is explicit
 * about that instead of pretending to be a single scrubbable recording: each
 * row names its timeline span and plays its own segment. A transcript jump
 * selects the chunk containing that position and seeks inside it.
 */
export const MeetingAudioPanel: React.FC<MeetingAudioPanelProps> = ({
  files,
  seekToMs,
}) => {
  const { t } = useTranslation();

  if (files.length === 0) {
    return (
      <p className="text-sm text-text/60">{t("meeting.detail.audio.none")}</p>
    );
  }

  const bySource = new Map<string, MeetingAudioFileDto[]>();
  for (const file of files) {
    const list = bySource.get(file.source) ?? [];
    list.push(file);
    bySource.set(file.source, list);
  }

  return (
    <div className="flex flex-col gap-4">
      {[...bySource.entries()].map(([source, chunks]) => (
        <section key={source} className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">
            {source === "microphone"
              ? t("meeting.live.levels.microphone")
              : t("meeting.live.levels.system")}
          </h3>
          <ul className="flex flex-col gap-2">
            {chunks.map((chunk) => {
              const containsSeek =
                seekToMs !== null &&
                seekToMs >= chunk.start_ms &&
                seekToMs < chunk.end_ms;
              const span = `${formatMeetingDuration(
                Math.floor(chunk.start_ms / 1000),
              )} – ${formatMeetingDuration(Math.floor(chunk.end_ms / 1000))}`;
              return (
                <li
                  key={`${chunk.source}-${chunk.sequence}`}
                  className={[
                    "flex flex-wrap items-center gap-3 rounded-lg border p-2",
                    containsSeek
                      ? "border-logo-primary bg-logo-primary/5"
                      : "border-mid-gray/20",
                  ].join(" ")}
                >
                  <span className="min-w-32 text-xs tabular-nums text-text/60">
                    {span}
                  </span>
                  <audio
                    className="h-9 flex-1"
                    controls
                    preload="none"
                    src={convertFileSrc(chunk.path)}
                    aria-label={t("meeting.detail.audio.playerLabel", {
                      span,
                    })}
                    ref={(node) => {
                      if (!node || !containsSeek || seekToMs === null) return;
                      const offset = (seekToMs - chunk.start_ms) / 1000;
                      const seek = () => {
                        // Guard against a metadata-less element: seeking past
                        // the end silently does nothing and would look broken.
                        if (Number.isFinite(node.duration)) {
                          node.currentTime = Math.min(
                            offset,
                            Math.max(0, node.duration - 0.05),
                          );
                        }
                      };
                      if (node.readyState > 0) seek();
                      else
                        node.addEventListener("loadedmetadata", seek, {
                          once: true,
                        });
                    }}
                  />
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
};

export default MeetingAudioPanel;
