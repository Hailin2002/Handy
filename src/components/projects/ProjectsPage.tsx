import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FolderPlus, Inbox, Trash2, Unlink } from "lucide-react";
import { commands, type MeetingSession, type Project } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";

/**
 * Projects page (plan §6.16–§6.18): project list, one-click creation and
 * per-project meeting membership plus the Eingang inbox of processed,
 * unassigned meetings. Unlinking never deletes the meeting.
 */
export const ProjectsPage: React.FC = () => {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessions, setSessions] = useState<MeetingSession[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [inboxIds, setInboxIds] = useState<string[]>([]);
  const [memberIds, setMemberIds] = useState<string[]>([]);

  const sessionById = useCallback(
    (id: string) => sessions.find((s) => s.id === id),
    [sessions],
  );

  const refreshSessions = useCallback(async () => {
    try {
      const result = await commands.meetingListSessions(null);
      if (result.status === "ok") setSessions(result.data);
    } catch (error) {
      console.error("Failed to load meetings:", error);
    }
  }, []);

  const refreshProjects = useCallback(async () => {
    try {
      const result = await commands.projectList();
      if (result.status === "ok") {
        setProjects(result.data);
        if (
          selectedId &&
          !result.data.some((project) => project.id === selectedId)
        ) {
          setSelectedId(null);
        }
      }
    } catch (error) {
      console.error("Failed to load projects:", error);
    }
  }, [selectedId]);

  const refreshInbox = useCallback(async () => {
    try {
      const result = await commands.projectInboxMeetingIds();
      if (result.status === "ok") setInboxIds(result.data);
    } catch (error) {
      console.error("Failed to load inbox:", error);
    }
  }, []);

  const refreshMembers = useCallback(async () => {
    if (!selectedId) {
      setMemberIds([]);
      return;
    }
    try {
      const result = await commands.projectMeetingIds(selectedId);
      if (result.status === "ok") setMemberIds(result.data);
    } catch (error) {
      console.error("Failed to load project members:", error);
    }
  }, [selectedId]);

  useEffect(() => {
    void refreshProjects();
    void refreshSessions();
    void refreshInbox();
  }, [refreshProjects, refreshSessions, refreshInbox]);

  useEffect(() => {
    void refreshMembers();
  }, [refreshMembers]);

  const handleCreate = async () => {
    const name = newName.trim();
    // Synchronous re-entry guard against same-frame double-clicks.
    if (!name || creating) return;
    setCreating(true);
    try {
      const result = await commands.projectCreate(name, null);
      if (result.status === "ok") {
        setNewName("");
        await refreshProjects();
        setSelectedId(result.data.id);
      } else {
        toast.error(t("projects.createError"), { description: result.error });
      }
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (projectId: string) => {
    try {
      const result = await commands.projectDelete(projectId);
      if (result.status === "ok") {
        await Promise.all([refreshProjects(), refreshInbox()]);
      } else {
        toast.error(t("projects.deleteError"), { description: result.error });
      }
    } catch (error) {
      console.error("Failed to delete project:", error);
      toast.error(t("projects.deleteError"));
    }
  };

  const assign = async (meetingId: string) => {
    if (!selectedId) return;
    try {
      const result = await commands.projectAssignMeeting(selectedId, meetingId);
      if (result.status === "ok") {
        await Promise.all([refreshMembers(), refreshInbox()]);
      }
    } catch (error) {
      console.error("Failed to assign meeting:", error);
    }
  };

  const unassign = async (meetingId: string) => {
    if (!selectedId) return;
    try {
      const result = await commands.projectUnassignMeeting(
        selectedId,
        meetingId,
      );
      if (result.status === "ok") {
        await Promise.all([refreshMembers(), refreshInbox()]);
      }
    } catch (error) {
      console.error("Failed to unassign meeting:", error);
    }
  };

  const selected = projects.find((p) => p.id === selectedId) ?? null;

  return (
    <section className="w-full max-w-xl" aria-label={t("nav.projects")}>
      <h2 className="text-lg font-semibold">{t("nav.projects")}</h2>

      <div className="mt-3 flex gap-2">
        <Input
          value={newName}
          onChange={(event) => setNewName(event.target.value)}
          placeholder={t("projects.namePlaceholder")}
          aria-label={t("projects.namePlaceholder")}
        />
        <Button onClick={handleCreate} disabled={!newName.trim() || creating}>
          <FolderPlus className="mr-1 h-4 w-4" aria-hidden="true" />
          {t("projects.create")}
        </Button>
      </div>

      <ul className="mt-3 space-y-1">
        {projects.map((project) => (
          <li key={project.id} className="flex items-center gap-1">
            <button
              type="button"
              className={`flex-1 rounded-md px-2 py-1.5 text-left text-sm ${
                project.id === selectedId
                  ? "bg-logo-primary/10 font-medium"
                  : "hover:bg-mid-gray/10"
              }`}
              onClick={() => setSelectedId(project.id)}
            >
              {project.name}
            </button>
            <Button
              size="sm"
              variant="ghost"
              aria-label={t("common.delete")}
              onClick={() => void handleDelete(project.id)}
            >
              <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
          </li>
        ))}
        {projects.length === 0 && (
          <li className="px-2 py-1 text-sm text-text/50">
            {t("projects.empty")}
          </li>
        )}
      </ul>

      {selected && (
        <div className="mt-5 rounded-lg border border-mid-gray/20 p-3">
          <h3 className="text-sm font-medium">{selected.name}</h3>

          <ul className="mt-2 space-y-1">
            {memberIds.map((meetingId) => {
              const session = sessionById(meetingId);
              return (
                <li
                  key={meetingId}
                  className="flex items-center justify-between gap-2 rounded-md bg-mid-gray/5 px-2 py-1 text-sm"
                >
                  <span>{session?.title ?? meetingId}</span>
                  <Button size="sm" variant="ghost" onClick={() => void unassign(meetingId)}>
                    <Unlink className="mr-1 h-3.5 w-3.5" aria-hidden="true" />
                    {t("projects.unlink")}
                  </Button>
                </li>
              );
            })}
            {memberIds.length === 0 && (
              <li className="text-sm text-text/50">{t("projects.noMeetings")}</li>
            )}
          </ul>

          {inboxIds.length > 0 && (
            <>
              <h4 className="mt-3 flex items-center gap-1 text-xs font-medium text-text/60">
                <Inbox className="h-3.5 w-3.5" aria-hidden="true" />
                {t("projects.assignFromInbox")}
              </h4>
              <div className="mt-1 flex flex-wrap gap-1">
                {inboxIds.map((meetingId) => (
                  <Button
                    key={meetingId}
                    size="sm"
                    variant="secondary"
                    onClick={() => void assign(meetingId)}
                  >
                    {sessionById(meetingId)?.title ?? meetingId.slice(0, 8)}
                  </Button>
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </section>
  );
};

export default ProjectsPage;
