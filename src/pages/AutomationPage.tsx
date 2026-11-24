import { useState } from "react";
import { Clock, Play, Plus, RefreshCw, Sparkles, Workflow as WorkflowIcon } from "lucide-react";
import { AppError, api, toAppError, type Workflow, type WorkflowRun } from "../lib/api";
import { formatDate } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { JsonView } from "../components/JsonView";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";

interface StepDefinition {
  agent_id?: string;
  prompt?: string;
  /** Étape « outil MCP » : serveur MCP existant et outil activé. */
  server_id?: string;
  mcp_tool?: string;
  arguments?: unknown;
}

export function AutomationPage() {
  const { t, locale, project } = useApp();
  const workflows = useAsync<Workflow[]>(async () => {
    if (!project) {
      return [];
    }
    return api.workflows.list(project.id);
  }, [project?.id]);
  const agents = useAsync(() => api.agents.list(), []);
  const createAction = useAction();
  const runAction = useAction();
  const [form, setForm] = useState({ name: "", interval: "15", steps: "" });
  const [stepsError, setStepsError] = useState<string | null>(null);
  const [formError, setFormError] = useState<AppError | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [messageTone, setMessageTone] = useState<"success" | "warning">("success");
  const [pendingRun, setPendingRun] = useState<string | null>(null);
  const [runs, setRuns] = useState<Record<string, WorkflowRun[]>>({});
  const [runsBusy, setRunsBusy] = useState<string | null>(null);
  const [runsError, setRunsError] = useState<Record<string, AppError>>({});
  const [visibleRuns, setVisibleRuns] = useState<Record<string, boolean>>({});
  const [lastRun, setLastRun] = useState<Record<string, WorkflowRun>>({});

  const parseSteps = (): StepDefinition[] | null => {
    const raw = form.steps.trim();
    if (raw.length === 0) {
      setStepsError(t("automation.steps_required"));
      return null;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed) || parsed.length === 0) {
        setStepsError(t("automation.steps_required"));
        return null;
      }
      const steps: StepDefinition[] = [];
      for (const entry of parsed) {
        if (typeof entry !== "object" || entry === null) {
          setStepsError(t("automation.steps_hint"));
          return null;
        }
        const candidate = entry as {
          agent_id?: unknown;
          prompt?: unknown;
          server_id?: unknown;
          mcp_tool?: unknown;
          arguments?: unknown;
        };
        if (typeof candidate.mcp_tool === "string" && candidate.mcp_tool.length > 0) {
          if (typeof candidate.server_id !== "string" || candidate.server_id.length === 0) {
            setStepsError(t("automation.steps_hint"));
            return null;
          }
          if (
            candidate.arguments !== undefined &&
            (typeof candidate.arguments !== "object" ||
              candidate.arguments === null ||
              Array.isArray(candidate.arguments))
          ) {
            setStepsError(t("automation.steps_hint"));
            return null;
          }
          steps.push({
            server_id: candidate.server_id,
            mcp_tool: candidate.mcp_tool,
            arguments: candidate.arguments,
          });
          continue;
        }
        if (typeof candidate.agent_id !== "string" || typeof candidate.prompt !== "string") {
          setStepsError(t("automation.steps_hint"));
          return null;
        }
        steps.push({ agent_id: candidate.agent_id, prompt: candidate.prompt });
      }
      setStepsError(null);
      return steps;
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      setStepsError(t("automation.invalid_json", { message: detail }));
      return null;
    }
  };

  const createWorkflow = async () => {
    if (!project) {
      return;
    }
    setMessage(null);
    setFormError(null);
    const steps = parseSteps();
    if (steps === null) {
      return;
    }
    const interval = Number(form.interval);
    if (!Number.isInteger(interval) || interval < 5) {
      setFormError(new AppError("invalid_input", t("automation.interval_hint"), true));
      return;
    }
    if (form.name.trim().length === 0) {
      setFormError(new AppError("invalid_input", t("automation.name_required"), true));
      return;
    }
    const created = await createAction.run(() =>
      api.workflows.upsert(project.id, form.name.trim(), interval, steps),
    );
    if (created) {
      setMessageTone("success");
      setMessage(t("automation.workflow_created"));
      setForm({ name: "", interval: form.interval, steps: "" });
      workflows.reload();
    }
  };

  const runNow = async (workflow: Workflow) => {
    setPendingRun(workflow.id);
    setMessage(null);
    const run = await runAction.run(() => api.workflows.runNow(workflow.id));
    setPendingRun(null);
    if (run) {
      setLastRun((previous) => ({ ...previous, [workflow.id]: run }));
      setMessageTone(run.status === "completed" ? "success" : "warning");
      setMessage(t("automation.run_result", { status: run.status }));
      setRuns((previous) => ({ ...previous, [workflow.id]: [run, ...(previous[workflow.id] ?? [])] }));
      setVisibleRuns((previous) => ({ ...previous, [workflow.id]: true }));
      workflows.reload();
    }
  };

  const loadRuns = async (workflow: Workflow) => {
    if (visibleRuns[workflow.id]) {
      setVisibleRuns((previous) => ({ ...previous, [workflow.id]: false }));
      return;
    }
    setRunsBusy(workflow.id);
    setRunsError((previous) => {
      const next = { ...previous };
      delete next[workflow.id];
      return next;
    });
    try {
      const history = await api.workflows.runs(workflow.id, 20);
      setRuns((previous) => ({ ...previous, [workflow.id]: history }));
      setVisibleRuns((previous) => ({ ...previous, [workflow.id]: true }));
    } catch (reason) {
      setRunsError((previous) => ({ ...previous, [workflow.id]: toAppError(reason) }));
    } finally {
      setRunsBusy(null);
    }
  };

  const insertTemplate = () => {
    const agent = (agents.data ?? []).find((entry) => entry.detected) ?? agents.data?.[0];
    if (!agent) {
      return;
    }
    setStepsError(null);
    setForm((current) => ({
      ...current,
      steps: JSON.stringify([{ agent_id: agent.id, prompt: "" }], null, 2),
    }));
  };

  if (!project) {
    return (
      <>
        <PageHeader title={t("automation.title")} />
        <ProjectGate title={t("automation.title")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("automation.title")}
        subtitle={t("automation.run_hint")}
        actions={
          <button type="button" className="ghost" onClick={() => workflows.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      {workflows.error ? <ErrorBanner error={workflows.error} onRetry={workflows.reload} /> : null}
      {agents.error ? <ErrorBanner error={agents.error} onRetry={agents.reload} /> : null}
      {createAction.error ? <ErrorBanner error={createAction.error} /> : null}
      {runAction.error ? <ErrorBanner error={runAction.error} /> : null}
      {formError ? <ErrorBanner error={formError} /> : null}
      {message ? <Notice tone={messageTone}>{message}</Notice> : null}

      <section className="card" aria-labelledby="automation-create">
        <div className="section-heading">
          <h2 id="automation-create">{t("automation.new_workflow")}</h2>
          <Sparkles size={17} aria-hidden="true" className="muted-icon" />
        </div>
        <div className="form-grid">
          <Field label={t("automation.name")} htmlFor="workflow-name" required>
            <input
              id="workflow-name"
              className="input"
              value={form.name}
              placeholder={t("automation.name_placeholder")}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
          </Field>
          <Field
            label={t("automation.interval_minutes")}
            htmlFor="workflow-interval"
            hint={t("automation.interval_hint")}
          >
            <input
              id="workflow-interval"
              className="input"
              type="number"
              min={5}
              value={form.interval}
              onChange={(event) => setForm({ ...form, interval: event.target.value })}
            />
          </Field>
        </div>
        <Field
          label={t("automation.steps_json")}
          htmlFor="workflow-steps"
          hint={t("automation.steps_hint")}
          error={stepsError}
          required
        >
          <textarea
            id="workflow-steps"
            className="input textarea mono"
            rows={7}
            spellCheck={false}
            value={form.steps}
            placeholder={'[{"agent_id": "…", "prompt": "…"}]'}
            onChange={(event) => {
              setStepsError(null);
              setForm({ ...form, steps: event.target.value });
            }}
          />
        </Field>
        <p className="muted small">{t("automation.agents_hint")}</p>
        <div className="button-row">
          <button
            type="button"
            className="ghost small"
            onClick={insertTemplate}
            disabled={!(agents.data ?? []).some((agent) => agent.detected)}
          >
            {t("automation.template")}
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => void createWorkflow()}
            disabled={createAction.busy}
            aria-busy={createAction.busy}
          >
            {createAction.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
            {t("common.create")}
          </button>
        </div>
      </section>

      {workflows.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : (workflows.data ?? []).length === 0 ? (
        <EmptyState
          icon={<WorkflowIcon size={22} aria-hidden="true" />}
          title={t("automation.no_workflows")}
          description={<p>{t("automation.no_workflows_hint")}</p>}
        />
      ) : (
        <div className="workflow-list">
          {(workflows.data ?? []).map((workflow) => {
            const stepCount = Array.isArray(workflow.steps) ? workflow.steps.length : 0;
            const run = lastRun[workflow.id];
            const history = runs[workflow.id] ?? [];
            const showHistory = visibleRuns[workflow.id] === true;
            return (
              <article className="card workflow-card" key={workflow.id}>
                <header className="provider-header">
                  <div>
                    <div className="provider-title">
                      <h2>{workflow.name}</h2>
                      <Badge tone={workflow.enabled ? "success" : "neutral"}>
                        {workflow.enabled ? t("automation.enabled_badge") : t("common.disabled")}
                      </Badge>
                      <Badge tone="neutral">
                        <Clock size={11} aria-hidden="true" />
                        {workflow.interval_minutes} {t("automation.minutes")}
                      </Badge>
                      <Badge tone="neutral">
                        {t("automation.steps_count", { count: stepCount })}
                      </Badge>
                    </div>
                    <p className="muted small">
                      {t("automation.last_run")} :{" "}
                      {workflow.last_run_at
                        ? formatDate(workflow.last_run_at, locale)
                        : t("common.none")}
                    </p>
                  </div>
                  <div className="button-row compact">
                    <button
                      type="button"
                      className="primary small"
                      onClick={() => void runNow(workflow)}
                      disabled={runAction.busy}
                      aria-busy={pendingRun === workflow.id}
                    >
                      {pendingRun === workflow.id ? <Spinner /> : <Play size={13} aria-hidden="true" />}
                      {runAction.busy && pendingRun === workflow.id
                        ? t("automation.run_running")
                        : t("automation.run_now")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void loadRuns(workflow)}
                      disabled={runsBusy !== null}
                    >
                      {runsBusy === workflow.id ? <Spinner /> : null}
                      {showHistory ? t("automation.hide_runs") : t("automation.load_runs")}
                    </button>
                  </div>
                </header>

                {run ? (
                  <Notice tone={run.status === "completed" ? "success" : "error"} title={t("automation.run_output")}>
                    <p>
                      {t("automation.status")} : {run.status}
                    </p>
                    {run.detail ? <pre className="code-block">{run.detail}</pre> : null}
                  </Notice>
                ) : null}

                {runsError[workflow.id] ? <ErrorBanner error={runsError[workflow.id]} /> : null}

                <details className="details">
                  <summary>{t("automation.steps")}</summary>
                  <JsonView value={workflow.steps} maxHeight={240} />
                </details>

                {showHistory ? (
                  <div className="run-history">
                    <h3 className="section-title">{t("automation.history")}</h3>
                    {history.length === 0 ? (
                      <p className="muted">{t("automation.no_runs")}</p>
                    ) : (
                      <div className="table-wrap">
                        <table className="table">
                          <thead>
                            <tr>
                              <th scope="col">{t("automation.status")}</th>
                              <th scope="col">{t("automation.started_at")}</th>
                              <th scope="col">{t("automation.finished_at")}</th>
                              <th scope="col">{t("automation.detail")}</th>
                            </tr>
                          </thead>
                          <tbody>
                            {history.map((entry) => (
                              <tr key={entry.id}>
                                <td>
                                  <Badge tone={entry.status === "completed" ? "success" : "danger"}>
                                    {entry.status === "completed"
                                      ? t("status.completed")
                                      : entry.status === "failed"
                                        ? t("status.failed")
                                        : entry.status}
                                  </Badge>
                                </td>
                                <td>{formatDate(entry.started_at, locale)}</td>
                                <td>
                                  {entry.finished_at
                                    ? formatDate(entry.finished_at, locale)
                                    : t("common.none")}
                                </td>
                                <td className="prewrap small">{entry.detail ?? "—"}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </div>
                ) : null}
              </article>
            );
          })}
        </div>
      )}
    </>
  );
}
