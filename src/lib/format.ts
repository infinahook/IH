const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"] as const;

/** Formate un nombre d'octets en unités binaires lisibles (0 B, 1.5 KiB…). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  if (bytes === 0) {
    return "0 B";
  }
  let exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  let value = bytes / 1024 ** exponent;
  // Arrondi qui peut faire passer 1023.99 KiB à 1 MiB.
  if (Number(value.toFixed(2)) >= 1024 && exponent < BYTE_UNITS.length - 1) {
    exponent += 1;
    value = bytes / 1024 ** exponent;
  }
  const maximumFractionDigits = exponent === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2;
  const rounded = Number(value.toFixed(maximumFractionDigits));
  return `${rounded} ${BYTE_UNITS[exponent]}`;
}

/** Formate une date ISO en date/heure locale lisible ; `—` si invalide. */
export function formatDate(value: string | null | undefined, locale: string): string {
  if (!value) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  const tag = locale.toLowerCase().startsWith("fr") ? "fr-FR" : "en-US";
  return new Intl.DateTimeFormat(tag, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

/** Raccourcit un identifiant technique (hash de commit, empreinte). */
export function shortHash(value: string, length = 10): string {
  if (value.length <= length) {
    return value;
  }
  return value.slice(0, length);
}

/** Compte les lignes d'un texte (un texte vide compte pour 0). */
export function countLines(text: string): number {
  if (text.length === 0) {
    return 0;
  }
  return text.split("\n").length;
}

/** Sérialise une valeur JSON avec indentation ; message lisible en cas d'échec. */
export function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2) ?? "null";
  } catch (error) {
    return String(error);
  }
}
