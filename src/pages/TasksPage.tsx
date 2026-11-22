import { useState } from "react";
import { ListChecks, Plus, RefreshCw, Sparkles, Workflow } from "lucide-react";
import { api, type PlanView, type Task, type TaskState } from "../lib/api";
import { formatDate } from "../lib/format";
import { useTauriEvent } from "../lib/events";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge, taskStateTone } from "../components/Badge";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";

const TASK_STATES: TaskState[] = [
  "pending",
  "ready",
  "running",
  "blocked",
  "completed",
  "failed",
  "cancelled",
];

const ROLES = ["lead", "worker", "reviewer"] as const;

export function TasksPage() {
  const { t, locale, project } = useApp();
  const tasks = useAsync<Task[]>(async () => {
    if (!project) {
      return [];
    }
    return api.tasks.list(project.id);
  }, [project?.id]);
  const agents = useAsync(() => api.agents.list(), []);
  const createAction = useAction();
  const stateAction = useAction();
  const planAction = useAction();

  const [form, setForm] = useState({
    title: "",
    description: "",
    role: "worker" as string,
    assignee: "",
  });
  const [planForm, setPlanForm] = useState({ goal: "", workers: 2 });
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [pendingTask, setPendingTask] = useState<string | null>(null);
  const taskEventError = useTauriEvent<{ task_id: string; state: TaskState }>(
    "ih://task-state",
    () => tasks.reload(),
  );

  const roleLabel = (role: string): string =>
    (ROLES as readonly string[]).includes(role) ? t(`agents.${role}`) : role;

  const taskTitles = new Map((tasks.data ?? []).map((task) => [task.id, task.title]));

  const createTask = async () => {
    if (!project || form.title.trim().length === 0) {
      return;
    }
    setMessage(null);
    const created = await createAction.run(() =>
      api.tasks.create(
        project.id,
        form.title.trim(),
        form.description.trim() || null,
        form.role,
        form.assignee || null,
      ),
    );
    if (created) {
      setMessage(t("tasks.created"));
      setForm({ title: "", description: "", role: "worker", assignee: "" });
      tasks.reload();
    }
  };

  const changeState = async (task: Task, next: TaskState) => {
    if (next === task.state) {
      return;
    }
    setPendingTask(task.id);
    setMessage(null);
    const done = await stateAction.run(async () => {
      await api.tasks.setState(task.id, next);
      return true;
    });
    setPendingTask(null);
    if (done) {
      setMessage(t("tasks.state_updated"));
      tasks.reload();
    }
  };

  const createPlan = async () => {
    if (!project || planForm.goal.trim().length === 0) {
      return;
    }
    setMessage(null);
    const created = await planAction.run(() =>
      api.tasks.plan(project.id, planForm.goal.trim(), planForm.workers),
    );
    if (created) {
      setPlan(created);
      setMessage(t("tasks.plan_created", { count: created.tasks.length }));
      tasks.reload();
    }
  };

  if (!project) {
    return (
      <>
        <PageHeader title={t("tasks.title")} />
        <ProjectGate title={t("tasks.title")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("tasks.title")}
        subtitle={t("tasks.create_hint")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => tasks.reload()}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <Badge tone="neutral">
              {t("tasks.tasks_count", { count: tasks.data?.length ?? 0 })}
            </Badge>
          </>
        }
      />

      {tasks.error ? <ErrorBanner error={tasks.error} onRetry={tasks.reload} /> : null}
      {agents.error ? <ErrorBanner error={agents.error} onRetry={agents.reload} /> : null}
      {createAction.error ? <ErrorBanner error={createAction.error} /> : null}
      {stateAction.error ? <ErrorBanner error={stateAction.error} /> : null}
      {planAction.error ? <ErrorBanner error={planAction.error} /> : null}
      {taskEventError ? <ErrorBanner error={taskEventError} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}

      <div className="grid grid-2">
        <section className="card" aria-labelledby="tasks-create">
          <div className="section-heading">
            <h2 id="tasks-create">{t("tasks.new_task")}</h2>
            <Plus size={17} aria-hidden="true" className="muted-icon" />
          </div>
          <Field label={t("tasks.title_label")} htmlFor="task-title" required>
            <input
              id="task-title"
              className="input"
              value={form.title}
              placeholder={t("tasks.title_placeholder")}
              onChange={(event) => setForm({ ...form, title: event.target.value })}
            />
          </Field>
          <Field
            label={t("tasks.description")}
            htmlFor="task-description"
            hint={t("common.optional")}
          >
            <textarea
              id="task-description"
              className="input textarea"
              rows={3}
              value={form.description}
              placeholder={t("tasks.description_placeholder")}
              onChange={(event) => setForm({ ...form, description: event.target.value })}
            />
          </Field>
          <div className="form-grid">
            <Field label={t("tasks.role")} htmlFor="task-role">
              <select
                id="task-role"
                className="input"
                value={form.role}
                onChange={(event) => setForm({ ...form, role: event.target.value })}
              >
                {ROLES.map((role) => (
                  <option key={role} value={role}>
                    {roleLabel(role)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("tasks.agent_optional")} htmlFor="task-assignee">
              <select
                id="task-assignee"
                className="input"
                value={form.assignee}
                onChange={(event) => setForm({ ...form, assignee: event.target.value })}
              >
                <option value="">{t("tasks.select_placeholder")}</option>
                {(agents.data ?? []).map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.label}
                    {agent.detected ? "" : ` — ${t("agents.missing")}`}
                  </option>
                ))}
              </select>
            </Field>
          </div>
          <div className="button-row">
            <button
              type="button"
              className="primary"
              onClick={() => void createTask()}
              disabled={createAction.busy || form.title.trim().length === 0}
              aria-busy={createAction.busy}
            >
              {createAction.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
              {t("tasks.new_task")}
            </button>
          </div>
        </section>

        <section className="card" aria-labelledby="tasks-plan">
          <div className="section-heading">
            <h2 id="tasks-plan">{t("tasks.create_plan")}</h2>
            <Sparkles size={17} aria-hidden="true" className="muted-icon" />
          </div>
          <Field label={t("tasks.plan_goal")} htmlFor="plan-goal" required>
            <textarea
              id="plan-goal"
              className="input textarea"
              rows={4}
              value={planForm.goal}
              onChange={(event) => setPlanForm({ ...planForm, goal: event.target.value })}
            />
          </Field>
          <Field
            label={t("tasks.plan_workers")}
            htmlFor="plan-workers"
            hint={t("tasks.workers_hint")}
          >
            <input
              id="plan-workers"
              className="input"
              type="number"
              min={1}
              max={4}
              value={planForm.workers}
              onChange={(event) =>
                setPlanForm({
                  ...planForm,
                  workers: Math.max(
                    1,
                    Math.min(4, Math.round(Number(event.target.value) || 1)),
                  ),
                })
              }
            />
          </Field>
          <p className="muted small">{t("tasks.plan_hint")}</p>
          <div className="button-row">
            <button
              type="button"
              className="primary"
              onClick={() => void createPlan()}
              disabled={planAction.busy || planForm.goal.trim().length === 0}
              aria-busy={planAction.busy}
            >
              {planAction.busy ? <Spinner /> : <Sparkles size={14} aria-hidden="true" />}
              {t("tasks.create_plan")}
            </button>
          </div>
        </section>
      </div>

      <section className="card" aria-labelledby="tasks-list">
        <div className="section-heading">
          <h2 id="tasks-list">{t("tasks.project_tasks")}</h2>
        </div>
        {tasks.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : tasks.data && tasks.data.length > 0 ? (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("tasks.title_label")}</th>
                  <th scope="col">{t("tasks.role")}</th>
                  <th scope="col">{t("tasks.assignee")}</th>
                  <th scope="col">{t("tasks.depends_on")}</th>
                  <th scope="col">{t("common.status")}</th>
                  <th scope="col">{t("tasks.set_state")}</th>
                  <th scope="col">{t("history.thread_created_at")}</th>
                </tr>
              </thead>
              <tbody>
                {tasks.data.map((task) => (
                  <tr key={task.id}>
                    <td>
                      <span className="strong">{task.title}</span>
                      {task.description ? (
                        <p className="muted small">{task.description}</p>
                      ) : null}
                    </td>
                    <td>
                      <Badge tone="neutral">{roleLabel(task.role)}</Badge>
                    </td>
                    <td>{task.assignee ?? t("tasks.unassigned")}</td>
                    <td>
                      {task.depends_on.length === 0
                        ? t("tasks.no_dependencies")
                        : task.depends_on
                            .map((id) => taskTitles.get(id) ?? id)
                            .join(", ")}
                    </td>
                    <td>
                      <Badge tone={taskStateTone(task.state)}>{t(`status.${task.state}`)}</Badge>
                    </td>
                    <td>
                      <select
                        className="input compact"
                        aria-label={`${t("tasks.set_state")} — ${task.title}`}
                        value={task.state}
                        disabled={stateAction.busy}
                        onChange={(event) =>
                          void changeState(task, event.target.value as TaskState)
                        }
                      >
                        {TASK_STATES.map((state) => (
                          <option key={state} value={state}>
                            {t(`status.${state}`)}
                          </option>
                        ))}
                      </select>
                      {pendingTask === task.id ? <Spinner /> : null}
                    </td>
                    <td>{formatDate(task.created_at, locale)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <EmptyState
            icon={<ListChecks size={22} aria-hidden="true" />}
            title={t("tasks.no_tasks")}
            description={<p>{t("tasks.no_tasks_hint")}</p>}
          />
        )}
      </section>

      <section className="card" aria-labelledby="plan-graph">
        <div className="section-heading">
          <h2 id="plan-graph">{t("tasks.plan_graph")}</h2>
          <Workflow size={17} aria-hidden="true" className="muted-icon" />
        </div>
        {!plan ? (
          <p className="muted">{t("tasks.plan_empty")}</p>
        ) : (
          <>
            <p className="muted small">{plan.goal}</p>
            <p className="muted small">{t("tasks.column_hint")}</p>
            <div className="plan-columns">
              {ROLES.map((role) => {
                const columnTasks = plan.tasks.filter((task) => task.role === role);
                return (
                  <div className="plan-column" key={role}>
                    <h3 className="section-title">
                      {roleLabel(role)}
                      <Badge tone="neutral">{columnTasks.length}</Badge>
                    </h3>
                    {columnTasks.length === 0 ? (
                      <p className="muted small">{t("common.none")}</p>
                    ) : (
                      <ul className="plan-task-list">
                        {columnTasks.map((task) => (
                          <li key={task.id} className="plan-task">
                            <span className="strong">{task.title}</span>
                            {task.description ? (
                              <p className="muted small">{task.description}</p>
                            ) : null}
                            <p className="small">
                              {t("tasks.depends_display", {
                                tasks:
                                  task.depends_on.length === 0
                                    ? t("tasks.no_dependencies")
                                    : task.depends_on
                                        .map(
                                          (id) =>
                                            plan.tasks.find((entry) => entry.id === id)?.title ?? id,
                                        )
                                        .join(", "),
                              })}
                            </p>
                            <p className="small muted">
                              {task.assignee ?? t("tasks.unassigned")}
                            </p>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                );
              })}
            </div>
          </>
        )}
      </section>
    </>
  );
}
