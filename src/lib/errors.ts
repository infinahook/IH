import type { AppError } from "./api";
import type { TFunction } from "./i18n";

export interface DescribedError {
  code: string;
  summary: string;
  detail: string;
  fixable: boolean;
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
