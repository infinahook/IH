import { CheckCircle2, Info, XCircle, AlertTriangle } from "lucide-react";
import type { ReactNode } from "react";

export type NoticeTone = "info" | "success" | "warning" | "error";

const ICONS = {
  info: Info,
  success: CheckCircle2,
  warning: AlertTriangle,
  error: XCircle,
} as const;

export function Notice({
  tone = "info",
  children,
  title,
}: {
  tone?: NoticeTone;
  children: ReactNode;
  title?: string;
}) {
  const Icon = ICONS[tone];
  return (
    <div
      className={`notice notice-${tone}`}
      role={tone === "error" ? "alert" : "status"}
    >
      <Icon size={16} aria-hidden="true" />
      <div>
        {title ? <strong>{title}</strong> : null}
        <div className="notice-body">{children}</div>
      </div>
    </div>
  );
}
