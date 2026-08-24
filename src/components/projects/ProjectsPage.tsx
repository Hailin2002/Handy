import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, FolderPlus, Inbox, Pencil, Trash2, Unlink, X } from "lucide-react";
import { commands, type MeetingSession, type Project, type ProjectTask } from "@/bindings";
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
  const [tasks, setTasks] = useState<ProjectTask[]>([]);
  const [newTaskTitle, setNewTaskTitle] = useState("");

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
      setTasks([]);
      return;
    }
    try {
      const [members, taskList] = await Promise.all([
        commands.projectMeetingIds(selectedId),
        commands.taskList(selectedId),
      ]);
      if (members.status === "ok") setMemberIds(members.data);
      if (taskList.status === "ok") setTasks(taskList.data);
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

  // -- Phase 10: task workflow ---------------------------------------------

  const addManualTask = async () => {
    if (!selectedId || !newTaskTitle.trim()) return;
    try {
      const result = await commands.taskCreateManual(
        selectedId,
        newTaskTitle.trim(),
        null,
      );
      if (result.status === "ok") {
        setNewTaskTitle("");
        await refreshMembers();
      }
    } catch (error) {
      console.error("Failed to create task:", error);
    }
  };

  /** Manual edits always win — the backend is insert-only for AI twins. */
  const decideTask = async (
    taskId: string,
    state: "accepted" | "rejected" | "edited",
  ) => {
    let title: string | null = null;
    if (state === "edited") {
      const answer = window.prompt(t("projects.taskEditPrompt"));
      if (answer === null || !answer.trim()) return;
      title = answer;
    }
    try {
      const result = await commands.taskDecide(taskId, state, title, null);
      if (result.status === "ok") await refreshMembers();
    } catch (error) {
      console.error("Failed to decide task:", error);
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

          {/* Phase 10 — task workflow */}
          <div className="mt-4 border-t border-mid-gray/15 pt-3">
            <h4 className="text-xs font-medium text-text/60">
              {t("projects.tasksHeading")}
            </h4>
            <ul className="mt-2 space-y-1">
              {tasks.map((task) => (
                <li
                  key={task.id}
                  className="flex items-center justify-between gap-2 rounded-md bg-mid-gray/5 px-2 py-1 text-sm"
                >
                  <span className={task.state === "rejected" ? "line-through opacity-50" : ""}>
                    {task.title}
                    {task.origin === "ai" && task.state === "suggested" && (
                      <span className="ml-1 text-xs text-text/50">
                        ({t("projects.taskSuggested")})
                      </span>
                    )}
                  </span>
                  <span className="flex gap-1">
                    {task.state === "suggested" && (
                      <>
                        <Button size="sm" variant="ghost" aria-label={t("projects.taskAccept")}
                          onClick={() => void decideTask(task.id, "accepted")}>
                          <Check className="h-3.5 w-3.5" aria-hidden="true" />
                        </Button>
                        <Button size="sm" variant="ghost" aria-label={t("projects.taskReject")}
                          onClick={() => void decideTask(task.id, "rejected")}>
                          <X className="h-3.5 w-3.5" aria-hidden="true" />
                        </Button>
                      </>
                    )}
                    {(task.state === "accepted" || task.state === "edited") && (
                      <Button size="sm" variant="ghost" aria-label={t("common.edit")}
                        onClick={() => void decideTask(task.id, "edited")}>
                        <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
                      </Button>
                    )}
                  </span>
                </li>
              ))}
              {tasks.length === 0 && (
                <li className="text-sm text-text/50">{t("projects.noTasks")}</li>
              )}
            </ul>
            <div className="mt-2 flex gap-1">
              <Input
                value={newTaskTitle}
                onChange={(event) => setNewTaskTitle(event.target.value)}
                placeholder={t("projects.newTaskPlaceholder")}
                aria-label={t("projects.newTaskPlaceholder")}
              />
              <Button
                size="sm"
                onClick={() => void addManualTask()}
                disabled={!newTaskTitle.trim()}
              >
                {t("common.add")}
              </Button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
};

export default ProjectsPage;
