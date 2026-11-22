import fr from "../../locales/fr.json";
import en from "../../locales/en.json";

export type Locale = "fr" | "en";
export type TranslationVars = Record<string, string | number>;
export type TFunction = (key: string, vars?: TranslationVars) => string;
export type Catalog = Record<string, unknown>;
export type Catalogs = Record<Locale, Catalog>;

export const LOCALES: readonly Locale[] = ["fr", "en"];
export const FALLBACK_LOCALE: Locale = "fr";

export const catalogs: Catalogs = {
  fr: fr as Catalog,
  en: en as Catalog,
};

/** Normalise une valeur de locale venue de la configuration ou du navigateur. */
export function normalizeLocale(value: string | null | undefined): Locale {
  const base = (value ?? "").trim().toLowerCase().split(/[-_]/)[0];
  return base === "en" ? "en" : FALLBACK_LOCALE;
}

/** Lit une clé pointée (`nav.projects`) dans un catalogue. */
export function lookup(catalog: Catalog, key: string): string | null {
  let current: unknown = catalog;
  for (const part of key.split(".")) {
    if (typeof current !== "object" || current === null || Array.isArray(current)) {
      return null;
    }
    current = (current as Record<string, unknown>)[part];
  }
  return typeof current === "string" ? current : null;
}

/** Remplace toutes les occurrences de `{nom}` par leur valeur. */
export function substitute(text: string, vars?: TranslationVars): string {
  if (!vars) {
    return text;
  }
  let result = text;
  for (const [name, value] of Object.entries(vars)) {
    result = result.split(`{${name}}`).join(String(value));
  }
  return result;
}

/**
 * Traduction pure : locale demandée → français → clé brute.
 * Exportée pour être testée avec des catalogues synthétiques.
 */
export function translateFrom(
  source: Catalogs,
  locale: Locale,
  key: string,
  vars?: TranslationVars,
): string {
  const primary = lookup(source[locale], key);
  const fallback = locale === FALLBACK_LOCALE ? null : lookup(source[FALLBACK_LOCALE], key);
  return substitute(primary ?? fallback ?? key, vars);
}

/** Traduction dans les catalogues réels embarqués dans le bundle. */
export function translate(locale: Locale, key: string, vars?: TranslationVars): string {
  return translateFrom(catalogs, locale, key, vars);
}

export function hasTranslation(locale: Locale, key: string): boolean {
  return lookup(catalogs[locale], key) !== null || lookup(catalogs[FALLBACK_LOCALE], key) !== null;
}

/** Construit la fonction `t` liée à une langue. */
export function createTranslator(locale: Locale): TFunction {
  return (key, vars) => translate(locale, key, vars);
}

/** Toutes les clés pointées d'un catalogue (utilisé par les tests de parité). */
export function flattenKeys(catalog: Catalog, prefix = ""): string[] {
  const keys: string[] = [];
  for (const [key, value] of Object.entries(catalog)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      keys.push(...flattenKeys(value as Catalog, path));
    } else {
      keys.push(path);
    }
  }
  return keys.sort();
}
