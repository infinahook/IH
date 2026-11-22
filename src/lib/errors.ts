import type { AppError } from "./api";
import type { TFunction } from "./i18n";

export interface DescribedError {
  code: string;
  summary: string;
  detail: string;
  fixable: boolean;
}

const FIXABLE_CODES: ReadonlySet<string> = new Set([
  "config_invalid",
  "invalid_input",
  "denied",
  "limit_reached",
  "provider_error",
  "network_error",
]);

/** Reproduit la classification `fixable` du noyau pour les erreurs arrivées par événement. */
export function errorIsFixable(code: string): boolean {
  return FIXABLE_CODES.has(code);
}

/**
 * Traduit une erreur IPC via `errors.<code>` ; le message brut du noyau reste
 * toujours visible. Un code inconnu est affiché tel quel.
 */
export function describeError(t: TFunction, error: AppError): DescribedError {
  const key = `errors.${error.code}`;
  const translated = t(key);
  const known = translated !== key;
  return {
    code: error.code,
    summary: known ? translated : error.message,
    detail: known ? error.message : "",
    fixable: error.fixable,
  };
}
