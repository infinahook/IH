import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, Package, RefreshCw, ShieldQuestion, Trash2 } from "lucide-react";
import {
  api,
  toAppError,
  type InstallReport,
  type SkillView,
} from "../lib/api";
import { shortHash } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

type SkillAction = "install" | "verify" | "enabled" | "remove";

export function SkillsPage() {
  const { t } = useApp();
  const skills = useAsync(() => api.skills.list(), []);
  const action = useAction();
  const [pending, setPending] = useState<{ id: string; kind: SkillAction } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [messageTone, setMessageTone] = useState<"success" | "warning">("success");
  const [toRemove, setToRemove] = useState<SkillView | null>(null);

  const runSkill = async <T,>(id: string, kind: SkillAction, task: () => Promise<T>) => {
    setMessage(null);
    setPending({ id, kind });
    try {
      return await action.run(task);
    } finally {
      setPending(null);
    }
  };

  const installSkill = async () => {
    let directory: string | string[] | null = null;
    try {
      directory = await open({ directory: true, multiple: false, title: t("skills.dialog_title") });
    } catch (reason) {
      action.setError(toAppError(reason));
      return;
    }
    if (typeof directory !== "string") {
      return;
    }
    const report = await runSkill<InstallReport>("", "install", () => api.skills.install(directory));
    if (report) {
      setMessageTone("success");
      setMessage(
        t("skills.install_ok", {
          id: report.id,
          version: report.version,
          files: report.files,
        }),
      );
      skills.reload();
    }
  };

  const verifySkill = async (skill: SkillView) => {
    const result = await runSkill(skill.id, "verify", () => api.skills.verify(skill.id));
    if (typeof result === "boolean") {
      setMessageTone(result ? "success" : "warning");
      setMessage(result ? t("skills.verify_ok") : t("skills.verify_failed"));
      skills.reload();
    }
  };

  const toggleSkill = async (skill: SkillView) => {
    const done = await runSkill(skill.id, "enabled", async () => {
      await api.skills.setEnabled(skill.id, !skill.enabled);
      return true;
    });
    if (done) {
      setMessageTone("success");
      setMessage(skill.enabled ? t("skills.enabled_off") : t("skills.enabled_on"));
      skills.reload();
    }
  };

  const removeSkill = async () => {
    if (!toRemove) {
      return;
    }
    const target = toRemove;
    const done = await runSkill(target.id, "remove", async () => {
      await api.skills.remove(target.id, target.version);
      return true;
    });
    if (done) {
      setToRemove(null);
      setMessageTone("success");
      setMessage(t("skills.removed"));
      skills.reload();
    }
  };

  const isBusy = (id: string, kind: SkillAction) =>
    pending !== null && pending.id === id && pending.kind === kind;

  return (
    <>
      <PageHeader
        title={t("skills.title")}
        subtitle={t("skills.install_hint")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => skills.reload()}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void installSkill()}
              disabled={pending !== null}
              aria-busy={isBusy("", "install")}
            >
              {isBusy("", "install") ? <Spinner /> : <Package size={15} aria-hidden="true" />}
              {t("skills.install_from_path")}
            </button>
          </>
        }
      />

      {skills.error ? <ErrorBanner error={skills.error} onRetry={skills.reload} /> : null}
      {action.error ? <ErrorBanner error={action.error} /> : null}
      {message ? <Notice tone={messageTone}>{message}</Notice> : null}
      <Notice tone="info">{t("skills.enable_scope_hint")}</Notice>

      {skills.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : skills.data && skills.data.length === 0 ? (
        <EmptyState
          icon={<Package size={22} aria-hidden="true" />}
          title={t("skills.no_skills")}
          description={<p>{t("skills.install_hint")}</p>}
          action={
            <button type="button" className="primary" onClick={() => void installSkill()}>
              {t("skills.install_from_path")}
            </button>
          }
        />
      ) : (
        <div className="skill-list">
          {(skills.data ?? []).map((skill) => {
            const verifiedTone =
              skill.verified === true ? "success" : skill.verified === false ? "danger" : "neutral";
            const verifiedLabel =
              skill.verified === true
                ? t("skills.verified_yes")
                : skill.verified === false
                  ? t("skills.verified_no")
                  : t("skills.verified_unknown");
            return (
              <article className="card skill-card" key={`${skill.id}-${skill.version}`}>
                <header className="skill-header">
                  <div>
                    <div className="provider-title">
                      <h2>{skill.name || skill.id}</h2>
                      <Badge tone="neutral">v{skill.version}</Badge>
                      <Badge tone={skill.enabled ? "success" : "neutral"}>
                        {skill.enabled ? t("common.enabled") : t("common.disabled")}
                      </Badge>
                    </div>
                    <p className="mono small">{skill.id}</p>
                  </div>
                  <Badge tone={verifiedTone}>
                    {skill.verified === true ? (
                      <CheckCircle2 size={12} aria-hidden="true" />
                    ) : (
                      <ShieldQuestion size={12} aria-hidden="true" />
                    )}
                    {verifiedLabel}
                  </Badge>
                </header>

                <dl className="detail-grid">
                  <div>
                    <dt>{t("skills.directory")}</dt>
                    <dd className="mono">{skill.dir}</dd>
                  </div>
                  <div>
                    <dt>{t("skills.source")}</dt>
                    <dd className="mono">{skill.source}</dd>
                  </div>
                  <div>
                    <dt>{t("skills.sha256")}</dt>
                    <dd className="mono" title={skill.sha256 ?? undefined}>
                      {skill.sha256 ? shortHash(skill.sha256, 24) : "—"}
                    </dd>
                  </div>
                  <div>
                    <dt>{t("skills.integrity")}</dt>
                    <dd>{verifiedLabel}</dd>
                  </div>
                </dl>

                <div className="button-row">
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void verifySkill(skill)}
                    disabled={pending !== null}
                    aria-busy={isBusy(skill.id, "verify")}
                  >
                    {isBusy(skill.id, "verify") ? <Spinner /> : null}
                    {t("skills.verify")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void toggleSkill(skill)}
                    disabled={pending !== null}
                    aria-busy={isBusy(skill.id, "enabled")}
                  >
                    {isBusy(skill.id, "enabled") ? <Spinner /> : null}
                    {skill.enabled ? t("common.disabled") : t("common.enabled")}
                  </button>
                  <button
                    type="button"
                    className="ghost small danger-text"
                    onClick={() => {
                      action.reset();
                      setToRemove(skill);
                    }}
                    disabled={pending !== null}
                  >
                    <Trash2 size={13} aria-hidden="true" />
                    {t("common.uninstall")}
                  </button>
                </div>
              </article>
            );
          })}
        </div>
      )}

      <ConfirmDialog
        open={toRemove !== null}
        title={t("skills.remove_confirm_title", { name: toRemove?.name ?? "" })}
        description={<p>{t("skills.remove_confirm_text")}</p>}
        confirmLabel={t("common.uninstall")}
        onCancel={() => setToRemove(null)}
        onConfirm={() => void removeSkill()}
        busy={isBusy(toRemove?.id ?? "", "remove")}
        danger
      />
    </>
  );
}
