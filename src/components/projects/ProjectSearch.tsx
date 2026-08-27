import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { commands, type SearchHit } from "@/bindings";
import { Input } from "@/components/ui/Input";

interface ProjectSearchProps {
  /** Narrows results to one project; omit to search everything. */
  projectId?: string | null;
  /** Called when the user opens a hit, if the parent can navigate to it. */
  onOpen?: (hit: SearchHit) => void;
}

/** Debounce so typing does not issue a query per keystroke. */
const DEBOUNCE_MS = 200;

/**
 * Full-text search over projects, meetings, transcripts, tasks and imported
 * chats (plan §9 "Suche").
 *
 * Every hit names the kind of thing it came from and shows the matching
 * excerpt, because the plan requires a result to carry its entity and its
 * position rather than being an opaque list of titles.
 */
export const ProjectSearch: React.FC<ProjectSearchProps> = ({
  projectId,
  onOpen,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);

  const run = useCallback(
    async (value: string) => {
      if (!value.trim()) {
        setHits([]);
        return;
      }
      setSearching(true);
      try {
        const result = await commands.searchEverything(
          value,
          projectId ?? null,
          50,
        );
        if (result.status === "ok") setHits(result.data);
      } catch (error) {
        // A failed search is not worth a toast on every keystroke; the empty
        // result plus the console entry is the honest outcome.
        console.error("Search failed:", error);
      } finally {
        setSearching(false);
      }
    },
    [projectId],
  );

  useEffect(() => {
    const handle = setTimeout(() => void run(query), DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [query, run]);

  return (
    <section className="flex flex-col gap-2" aria-label={t("search.label")}>
      <div className="flex items-center gap-2">
        <Search className="h-4 w-4 shrink-0 text-text/50" aria-hidden="true" />
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("search.placeholder")}
          aria-label={t("search.label")}
        />
      </div>

      {/* Announced politely so a screen reader hears the count without losing
          the caret in the field (§6.14). */}
      <p className="text-xs text-text/50" role="status" aria-live="polite">
        {query.trim()
          ? searching
            ? t("search.searching")
            : t("search.results", { count: hits.length })
          : ""}
      </p>

      {hits.length > 0 && (
        <ul className="flex max-h-72 flex-col gap-1 overflow-y-auto">
          {hits.map((hit) => (
            <li key={`${hit.entity_kind}-${hit.entity_id}`}>
              <button
                type="button"
                onClick={() => onOpen?.(hit)}
                disabled={!onOpen}
                className="flex w-full flex-col gap-0.5 rounded-md border border-mid-gray/20 p-2 text-left hover:bg-mid-gray/5 disabled:cursor-default disabled:hover:bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
              >
                <span className="flex items-center gap-2 text-xs text-text/50">
                  {t(`search.kind.${hit.entity_kind}`)}
                  {hit.title && <span className="truncate">· {hit.title}</span>}
                </span>
                <span className="text-sm text-text/80">{hit.snippet}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
};

export default ProjectSearch;
