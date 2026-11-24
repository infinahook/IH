/** Étape d'un workflow, telle que saisie dans l'interface. */
export interface StepDefinition {
  agent_id: string;
  prompt: string;
}

/** Erreurs de saisie des étapes, traduites par la page. */
export type StepsError = "steps_required" | "steps_hint" | "invalid_json";

export interface ParsedSteps {
  steps: StepDefinition[] | null;
  error: StepsError | null;
  /** Message technique du parseur JSON (pour `invalid_json`). */
  detail: string | null;
}

/** Sérialise les étapes d'un workflow en JSON lisible pour l'édition. */
export function formatSteps(steps: unknown): string {
  return JSON.stringify(steps, null, 2);
}

/**
 * Analyse la saisie JSON des étapes d'un workflow.
 *
 * Contrat identique à la validation Rust (`workflow_upsert`) : tableau non vide
 * d'objets `{ agent_id, prompt }`, tous deux des chaînes.
 */
export function parseSteps(raw: string): ParsedSteps {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    return { steps: null, error: "steps_required", detail: null };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch (error) {
    return {
      steps: null,
      error: "invalid_json",
      detail: error instanceof Error ? error.message : String(error),
    };
  }
  if (!Array.isArray(parsed) || parsed.length === 0) {
    return { steps: null, error: "steps_required", detail: null };
  }
  const steps: StepDefinition[] = [];
  for (const entry of parsed) {
    if (typeof entry !== "object" || entry === null) {
      return { steps: null, error: "steps_hint", detail: null };
    }
    const candidate = entry as { agent_id?: unknown; prompt?: unknown };
    if (typeof candidate.agent_id !== "string" || typeof candidate.prompt !== "string") {
      return { steps: null, error: "steps_hint", detail: null };
    }
    steps.push({ agent_id: candidate.agent_id, prompt: candidate.prompt });
  }
  return { steps, error: null, detail: null };
}
