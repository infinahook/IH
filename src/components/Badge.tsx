import type { ReactNode } from "react";

export type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

export function Badge({
  children,
  tone = "neutral",
  title,
}: {
  children: ReactNode;
  tone?: BadgeTone;
  title?: string;
}) {
  return (
    <span className={`badge badge-${tone}`} title={title}>
      {children}
    </span>
  );
}

export function riskTone(risk: "low" | "medium" | "high" | "critical"): BadgeTone {
  switch (risk) {
    case "low":
      return "success";
    case "medium":
      return "warning";
    case "high":
    case "critical":
      return "danger";
  }
}

export function agentStateTone(state: string): BadgeTone {
  switch (state) {
    case "running":
    case "starting":
    case "ready":
      return "accent";
    case "completed":
      return "success";
    case "waiting_approval":
      return "warning";
    case "failed":
      return "danger";
    case "cancelled":
      return "neutral";
    default:
      return "neutral";
  }
}

export function taskStateTone(state: string): BadgeTone {
  switch (state) {
    case "running":
    case "ready":
      return "accent";
    case "completed":
      return "success";
    case "blocked":
    case "pending":
      return "warning";
    case "failed":
      return "danger";
    case "cancelled":
      return "neutral";
    default:
      return "neutral";
  }
}
