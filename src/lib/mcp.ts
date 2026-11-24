/** Aides pures pour la page MCP (testées sans Tauri). */

/** Champ déduit du schéma JSON d'entrée d'un outil. */
export interface SchemaField {
  name: string;
  type: string;
  required: boolean;
}

/** Extrait les champs lisibles d'un schéma JSON d'outil MCP. */
export function schemaFields(schema: unknown): SchemaField[] {
  if (typeof schema !== "object" || schema === null) {
    return [];
  }
  const record = schema as { properties?: unknown; required?: unknown };
  const properties =
    typeof record.properties === "object" && record.properties !== null
      ? (record.properties as Record<string, unknown>)
      : {};
  const required = Array.isArray(record.required)
    ? record.required.filter((entry): entry is string => typeof entry === "string")
    : [];
  return Object.entries(properties).map(([name, value]) => {
    const type =
      typeof value === "object" && value !== null && typeof (value as { type?: unknown }).type === "string"
        ? String((value as { type: unknown }).type)
        : "any";
    return { name, type, required: required.includes(name) };
  });
}

/** Sévérité croissante : plus le rang est haut, plus l'outil est sensible. */
const RISK_ORDER: Record<string, number> = {
  low: 0,
  medium: 1,
  high: 2,
  critical: 3,
};

/** Vrai si l'outil est au moins aussi sensible que le seuil. */
export function riskAtLeast(risk: string, threshold: string): boolean {
  return (RISK_ORDER[risk] ?? 1) >= (RISK_ORDER[threshold] ?? 1);
}

/** Classe CSS de badge adaptée au risque. */
export function riskTone(risk: string): "success" | "warning" | "danger" | "neutral" {
  switch (risk) {
    case "low":
      return "success";
    case "medium":
      return "warning";
    case "high":
      return "danger";
    case "critical":
      return "danger";
    default:
      return "neutral";
  }
}

/** Libellé court d'un statut de connexion serveur. */
export function statusTone(status: string, connected: boolean): "success" | "warning" | "danger" | "neutral" {
  if (connected) {
    return "success";
  }
  switch (status) {
    case "connected":
      return "success";
    case "connecting":
      return "warning";
    case "error":
      return "danger";
    default:
      return "neutral";
  }
}

/** Résumé compact d'un résultat d'outil (jamais plus long que `max`). */
export function summarizeResult(text: string, max = 4000): string {
  if (text.length <= max) {
    return text;
  }
  return `${text.slice(0, max)}…`;
}
