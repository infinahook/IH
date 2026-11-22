/**
 * Mentions `@chemin` du compositeur et chips de contexte.
 *
 * Toutes les opérations sont pures : le composant se contente de refléter le
 * résultat du noyau (`files_match`, `context_prepare`). Aucun chemin n'est
 * inventé ici.
 */

/** Jeton `@chemin` en cours de frappe, à la fin du texte (ou du curseur). */
export interface MentionToken {
  /** Index du caractère `@`. */
  start: number;
  /** Index de fin du texte analysé (le curseur). */
  end: number;
  /** Texte saisi après `@` (peut être vide). */
  query: string;
}

const MENTION_PATTERN = /(?:^|\s)@([^\s@]*)$/;

/**
 * Détecte un jeton `@chemin` en cours de frappe. Le jeton doit toucher la fin
 * du texte analysé : une mention déjà terminée (suivie d'un espace) n'ouvre
 * pas la liste déroulante.
 */
export function findMentionToken(text: string, caret: number = text.length): MentionToken | null {
  const position = Math.max(0, Math.min(caret, text.length));
  const before = text.slice(0, position);
  const match = MENTION_PATTERN.exec(before);
  if (!match) {
    return null;
  }
  const query = match[1] ?? "";
  return {
    start: position - query.length - 1,
    end: position,
    query,
  };
}

export interface InsertMentionResult {
  text: string;
  /** Position du curseur après insertion (fin de la mention + espace). */
  caret: number;
}

/**
 * Remplace le jeton en cours de frappe par `@chemin ` et place le curseur
 * après l'espace. Le texte hors jeton n'est jamais modifié.
 */
export function insertMention(
  text: string,
  token: MentionToken,
  path: string,
): InsertMentionResult {
  const clean = path.trim();
  if (clean.length === 0) {
    return { text, caret: token.end };
  }
  const inserted = `@${clean} `;
  const next = `${text.slice(0, token.start)}${inserted}${text.slice(token.end)}`;
  return { text: next, caret: token.start + inserted.length };
}

/** Index du premier caractère de chaque ligne, utilisé pour replacer le curseur. */
export function caretFromOffset(text: string, offset: number): number {
  return Math.max(0, Math.min(offset, text.length));
}

/** Chip de contexte affiché au-dessus du compositeur. */
export interface ContextChip {
  path: string;
  /** Taille disque en octets, quand `workspace_meta` l'a renvoyée. */
  size: number | null;
}

/**
 * Ajoute un chip sans doublon. Un chemin déjà présent conserve sa position et
 * met à jour sa taille si une valeur réelle est fournie.
 */
export function addChip(chips: readonly ContextChip[], chip: ContextChip): ContextChip[] {
  const path = chip.path.trim();
  if (path.length === 0) {
    return [...chips];
  }
  const existing = chips.findIndex((entry) => entry.path === path);
  if (existing < 0) {
    return [...chips, { path, size: chip.size }];
  }
  if (chip.size === null || chips[existing].size === chip.size) {
    return [...chips];
  }
  const next = [...chips];
  next[existing] = { ...next[existing], size: chip.size };
  return next;
}

/** Supprime un chip ; le texte du message n'est jamais touché. */
export function removeChip(chips: readonly ContextChip[], path: string): ContextChip[] {
  return chips.filter((chip) => chip.path !== path);
}

/** Vide complètement le contexte. */
export function clearChips(): ContextChip[] {
  return [];
}

/** Chemins transmis à `context_prepare` puis à `chat_send`, dans l'ordre des chips. */
export function extractContextPaths(chips: readonly ContextChip[]): string[] {
  const seen = new Set<string>();
  const paths: string[] = [];
  for (const chip of chips) {
    const path = chip.path.trim();
    if (path.length === 0 || seen.has(path)) {
      continue;
    }
    seen.add(path);
    paths.push(path);
  }
  return paths;
}

/** Vrai si un compteur d'octets tient dans le budget affiché. */
export function fitsBudget(bytes: number, maxBytes: number): boolean {
  if (!Number.isFinite(bytes) || !Number.isFinite(maxBytes) || maxBytes < 0) {
    return false;
  }
  return bytes <= maxBytes;
}
