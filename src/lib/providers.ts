import type { ProviderKind, ProviderView } from "./api";

// ------------------------------------------------------------- protocoles

/** Les quatre protocoles acceptés par le noyau, dans l'ordre du formulaire. */
export const PROVIDER_KINDS: readonly ProviderKind[] = [
  "openai_compatible",
  "anthropic",
  "google",
  "ollama",
];

/** Un service local (Ollama) n'exige pas de clé d'API. */
export function providerRequiresSecret(kind: ProviderKind): boolean {
  return kind !== "ollama";
}

// ------------------------------------------------------- validation du slug

export type ProviderSlugError = "empty" | "length" | "characters" | "start";

/**
 * Reproduit exactement la validation du noyau (`validate_provider_id`) :
 * 2 à 32 caractères, minuscules a-z, chiffres 0-9, `-` et `_`, premier
 * caractère alphanumérique. Renvoie le code d'erreur, ou `null` si valide.
 */
export function providerSlugError(value: string): ProviderSlugError | null {
  const id = value.trim();
  if (id.length === 0) {
    return "empty";
  }
  if (id.length < 2 || id.length > 32) {
    return "length";
  }
  if (!/^[a-z0-9_-]+$/.test(id)) {
    return "characters";
  }
  if (!/^[a-z0-9]/.test(id)) {
    return "start";
  }
  return null;
}

export function isValidProviderSlug(value: string): boolean {
  return providerSlugError(value) === null;
}

// ---------------------------------------------------------- validation d'URL

export type BaseUrlError = "empty" | "scheme" | "malformed" | "remote_http";

/**
 * Hôtes acceptés pour un endpoint `http://` en clair : la boucle locale et les
 * plages privées usuelles. Tout le reste exige `https://`.
 */
export function isLocalHostname(hostname: string): boolean {
  const host = hostname.trim().toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
  if (host.length === 0) {
    return false;
  }
  if (host === "localhost" || host === "::1" || host === "0.0.0.0") {
    return true;
  }
  if (/^127\./.test(host)) {
    return true;
  }
  if (/^10\./.test(host)) {
    return true;
  }
  if (/^192\.168\./.test(host)) {
    return true;
  }
  // 172.16.0.0 – 172.31.255.255
  return /^172\.(1[6-9]|2\d|3[01])\./.test(host);
}

function parseUrl(value: string): URL | null {
  try {
    return new URL(value);
  } catch {
    return null;
  }
}

/**
 * Valide l'URL de base d'un fournisseur :
 * `https://` partout, `http://` uniquement vers un hôte local.
 */
export function baseUrlError(value: string): BaseUrlError | null {
  const url = value.trim();
  if (url.length === 0) {
    return "empty";
  }
  if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(url)) {
    return "scheme";
  }
  const parsed = parseUrl(url);
  if (!parsed) {
    return "malformed";
  }
  const protocol = parsed.protocol.toLowerCase();
  if (protocol !== "http:" && protocol !== "https:") {
    return "scheme";
  }
  if (parsed.hostname.length === 0) {
    return "malformed";
  }
  if (protocol === "http:" && !isLocalHostname(parsed.hostname)) {
    return "remote_http";
  }
  return null;
}

export function isValidBaseUrl(value: string): boolean {
  return baseUrlError(value) === null;
}

// -------------------------------------------------------- en-têtes HTTP

export type HeaderIssueCode = "missing_colon" | "empty_name" | "invalid_name" | "duplicate";

export interface HeaderIssue {
  /** Numéro de ligne dans la zone de texte, à partir de 1. */
  line: number;
  code: HeaderIssueCode;
}

export interface ParsedHeaders {
  headers: Record<string, string>;
  issues: HeaderIssue[];
}

// Nom d'en-tête HTTP valide (RFC 7230, token) : pas d'espace, pas de `:`.
const HEADER_NAME = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

/**
 * Analyse un éditeur « un en-tête par ligne » au format `Nom: valeur`.
 * Les lignes vides sont ignorées ; la valeur peut contenir des `:`.
 */
export function parseHeaders(text: string): ParsedHeaders {
  const headers: Record<string, string> = {};
  const issues: HeaderIssue[] = [];
  const seen = new Set<string>();
  const lines = text.split(/\r?\n/);

  lines.forEach((raw, index) => {
    const line = raw.trim();
    if (line.length === 0) {
      return;
    }
    const lineNumber = index + 1;
    const separator = line.indexOf(":");
    if (separator < 0) {
      issues.push({ line: lineNumber, code: "missing_colon" });
      return;
    }
    const name = line.slice(0, separator).trim();
    const value = line.slice(separator + 1).trim();
    if (name.length === 0) {
      issues.push({ line: lineNumber, code: "empty_name" });
      return;
    }
    if (!HEADER_NAME.test(name)) {
      issues.push({ line: lineNumber, code: "invalid_name" });
      return;
    }
    const lower = name.toLowerCase();
    if (seen.has(lower)) {
      issues.push({ line: lineNumber, code: "duplicate" });
      return;
    }
    seen.add(lower);
    headers[name] = value;
  });

  return { headers, issues };
}

/** Sérialise des en-têtes dans l'éditeur `Nom: valeur`, triés par nom. */
export function serializeHeaders(headers: Record<string, string> | null | undefined): string {
  if (!headers) {
    return "";
  }
  return Object.entries(headers)
    .map(([name, value]) => [name.trim(), value.trim()])
    .filter(([name]) => name.length > 0)
    .sort(([a], [b]) => a.localeCompare(b, "en", { sensitivity: "base" }))
    .map(([name, value]) => `${name}: ${value}`)
    .join("\n");
}

/** Liste d'en-têtes affichable (nom, valeur) triée, sans perdre la casse d'origine. */
export function headerList(
  headers: Record<string, string> | null | undefined,
): Array<{ name: string; value: string }> {
  if (!headers) {
    return [];
  }
  return Object.entries(headers)
    .filter(([name]) => name.trim().length > 0)
    .map(([name, value]) => ({ name: name.trim(), value: value.trim() }))
    .sort((a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }));
}

// -------------------------------------------------- test de connexion

export type ConnectionStatus =
  | "ok"
  | "auth_failed"
  | "unreachable"
  | "unsupported_protocol"
  | "model_unavailable"
  | "timeout"
  | "error";

export const CONNECTION_STATUSES: readonly ConnectionStatus[] = [
  "ok",
  "auth_failed",
  "unreachable",
  "unsupported_protocol",
  "model_unavailable",
  "timeout",
  "error",
];

/** Tout statut inconnu du noyau retombe sur « error » plutôt que d'inventer. */
export function normalizeConnectionStatus(status: string): ConnectionStatus {
  const value = status.trim().toLowerCase();
  return (CONNECTION_STATUSES as readonly string[]).includes(value)
    ? (value as ConnectionStatus)
    : "error";
}

/** Clé i18n du libellé français du statut. */
export function connectionStatusKey(status: string): string {
  return `providers.status_${normalizeConnectionStatus(status)}`;
}

export type ConnectionTone = "success" | "warning" | "danger";

export function connectionStatusTone(status: string): ConnectionTone {
  switch (normalizeConnectionStatus(status)) {
    case "ok":
      return "success";
    case "unsupported_protocol":
    case "model_unavailable":
    case "timeout":
      return "warning";
    case "auth_failed":
    case "unreachable":
    case "error":
      return "danger";
  }
}

/** Tone compatible avec `Notice` (qui attend `error`, pas `danger`). */
export function connectionNoticeTone(status: string): "success" | "warning" | "error" {
  const tone = connectionStatusTone(status);
  return tone === "danger" ? "error" : tone;
}

// --------------------------------------------------------- liste / tri

/**
 * Tri de la page Fournisseurs : les intégrés d'abord, puis par nom
 * alphabétique (sans tenir compte de la casse), puis par identifiant.
 */
export function sortProviders<T extends Pick<ProviderView, "id" | "label" | "custom">>(
  list: readonly T[],
): T[] {
  return [...list].sort((a, b) => {
    if (a.custom !== b.custom) {
      return a.custom ? 1 : -1;
    }
    const byLabel = a.label.localeCompare(b.label, "fr", { sensitivity: "base" });
    if (byLabel !== 0) {
      return byLabel;
    }
    return a.id.localeCompare(b.id);
  });
}

/** Sépare les fournisseurs activés de ceux qui sont indisponibles dans les sélecteurs. */
export function splitProvidersByAvailability<T extends Pick<ProviderView, "enabled">>(
  list: readonly T[],
): { enabled: T[]; disabled: T[] } {
  return {
    enabled: list.filter((entry) => entry.enabled),
    disabled: list.filter((entry) => !entry.enabled),
  };
}
