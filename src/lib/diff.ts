/**
 * Diff avant/après préparé par le noyau (`diff_prepare`).
 *
 * Le calcul vient de Rust ; ce module ne fait que compter, libeller et
 * présenter les hunks. Aucun diff n'est recalculé côté interface.
 */

export type DiffLineKind = "context" | "add" | "remove";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  old_line: number | null;
  new_line: number | null;
}

export interface DiffHunk {
  old_start: number;
  old_len: number;
  new_start: number;
  new_len: number;
  lines: DiffLine[];
}

export interface LineDiff {
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
  truncated: boolean;
  old_lines: number;
  new_lines: number;
}

export interface DiffCounts {
  additions: number;
  deletions: number;
}

/** Compte les ajouts et suppressions réels d'un hunk (les lignes de contexte sont ignorées). */
export function countHunkChanges(hunk: DiffHunk): DiffCounts {
  let additions = 0;
  let deletions = 0;
  for (const line of hunk.lines) {
    if (line.kind === "add") {
      additions += 1;
    } else if (line.kind === "remove") {
      deletions += 1;
    }
  }
  return { additions, deletions };
}

/** Compte les changements de tous les hunks (source de vérité pour l'affichage). */
export function countDiffChanges(diff: Pick<LineDiff, "hunks">): DiffCounts {
  return diff.hunks.reduce<DiffCounts>(
    (total, hunk) => {
      const counts = countHunkChanges(hunk);
      return {
        additions: total.additions + counts.additions,
        deletions: total.deletions + counts.deletions,
      };
    },
    { additions: 0, deletions: 0 },
  );
}

const MINUS = "\u2212";

/** Libellé compact des compteurs, par exemple `+3 −1`. */
export function diffStatsLabel(additions: number, deletions: number): string {
  const add = Number.isFinite(additions) && additions > 0 ? Math.floor(additions) : 0;
  const del = Number.isFinite(deletions) && deletions > 0 ? Math.floor(deletions) : 0;
  return `+${add} ${MINUS}${del}`;
}

/** En-tête d'un hunk au format unifié, par exemple `@@ -1,4 +1,6 @@`. */
export function hunkLabel(hunk: DiffHunk): string {
  return `@@ -${hunk.old_start},${hunk.old_len} +${hunk.new_start},${hunk.new_len} @@`;
}

/** Vrai si le noyau a simplifié le diff faute de budget de calcul. */
export function isDiffTruncated(diff: Pick<LineDiff, "truncated">): boolean {
  return diff.truncated === true;
}

/** Vrai si aucun ajout ni suppression n'est présent. */
export function isDiffEmpty(diff: Pick<LineDiff, "hunks" | "additions" | "deletions">): boolean {
  return diff.additions === 0 && diff.deletions === 0 && diff.hunks.length === 0;
}

export interface DiffSummary {
  additions: number;
  deletions: number;
  truncated: boolean;
  hunks: number;
  oldLines: number;
  newLines: number;
  label: string;
}

/** Résumé cohérent pour la barre du diff : compteurs recalculés et libellé. */
export function summarizeDiff(diff: LineDiff): DiffSummary {
  const counts = countDiffChanges(diff);
  return {
    additions: counts.additions,
    deletions: counts.deletions,
    truncated: isDiffTruncated(diff),
    hunks: diff.hunks.length,
    oldLines: diff.old_lines,
    newLines: diff.new_lines,
    label: diffStatsLabel(counts.additions, counts.deletions),
  };
}

/** Numéros de ligne affichés à gauche (ancien) et à droite (nouveau). */
export function lineNumbers(line: DiffLine): { old: number | null; new: number | null } {
  return { old: line.old_line, new: line.new_line };
}
