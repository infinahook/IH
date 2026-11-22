import type { GitStatus, StatusEntry } from "./api";

export type GitChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "untracked"
  | "conflict"
  | "ignored"
  | "other";

export interface GitStatusRow {
  path: string;
  /** Code porcelain de la colonne index (vide si non concerné). */
  stagedCode: string;
  /** Code porcelain de la colonne arbre de travail (vide si non concerné). */
  worktreeCode: string;
  staged: boolean;
  worktree: boolean;
  stagedKind: GitChangeKind;
  worktreeKind: GitChangeKind;
}

const CODE_KINDS: Record<string, GitChangeKind> = {
  A: "added",
  M: "modified",
  T: "modified",
  D: "deleted",
  R: "renamed",
  C: "added",
  U: "conflict",
  "?": "untracked",
  "!": "ignored",
};

/** Convertit un code porcelain (`A`, `M`, `?`…) en catégorie affichable. */
export function changeKindFromCode(code: string): GitChangeKind {
  return CODE_KINDS[code] ?? "other";
}

function isPresent(code: string): boolean {
  return code !== " " && code !== "";
}

/**
 * Transforme un `GitStatus` en lignes affichables par les onglets indexé /
 * non indexé. Un fichier `MM` apparaît dans les deux onglets avec des
 * catégories distinctes.
 */
export function gitStatusRows(status: Pick<GitStatus, "entries">): GitStatusRow[] {
  return status.entries
    .map((entry: StatusEntry): GitStatusRow => {
      const untracked = entry.index === "?" && entry.worktree === "?";
      const pair = `${entry.index}${entry.worktree}`;
      const conflicted =
        entry.index === "U" ||
        entry.worktree === "U" ||
        ["AA", "DD", "AU", "UA", "DU", "UD", "UU"].includes(pair);
      const ignored = entry.index === "!" || entry.worktree === "!";
      const staged = !untracked && !ignored && (entry.staged || isPresent(entry.index));
      const worktree = !ignored && (untracked || isPresent(entry.worktree));
      const stagedKind: GitChangeKind = conflicted
        ? "conflict"
        : untracked
          ? "untracked"
          : changeKindFromCode(entry.index);
      const worktreeKind: GitChangeKind = conflicted
        ? "conflict"
        : untracked
          ? "untracked"
          : changeKindFromCode(entry.worktree);
      return {
        path: entry.path,
        stagedCode: staged ? entry.index.trim() : "",
        worktreeCode: worktree ? entry.worktree.trim() : "",
        staged,
        worktree,
        stagedKind,
        worktreeKind,
      };
    })
    .sort((left, right) => left.path.localeCompare(right.path));
}

export function stagedRows(rows: GitStatusRow[]): GitStatusRow[] {
  return rows.filter((row) => row.staged);
}

export function unstagedRows(rows: GitStatusRow[]): GitStatusRow[] {
  return rows.filter((row) => row.worktree);
}
