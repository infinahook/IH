import { FolderOpen } from "lucide-react";
import { useApp } from "../lib/store";
import { EmptyState } from "./EmptyState";

export function ProjectGate({ title }: { title?: string }) {
  const { t, navigate } = useApp();
  return (
    <EmptyState
      icon={<FolderOpen size={22} aria-hidden="true" />}
      title={title ?? t("gate.title")}
      description={
        <>
          <p>{t("gate.text")}</p>
          <p>{t("gate.hint")}</p>
        </>
      }
      action={
        <button type="button" className="primary" onClick={() => navigate("projects")}>
          {t("gate.action")}
        </button>
      }
    />
  );
}
