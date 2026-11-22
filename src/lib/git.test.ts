import { describe, expect, it } from "vitest";
import type { GitStatus, StatusEntry } from "./api";
import { changeKindFromCode, gitStatusRows, stagedRows, unstagedRows } from "./git";

function status(entries: StatusEntry[]): GitStatus {
  return { branch: "main", entries, ahead: 0, behind: 0, clean: entries.length === 0 };
}

function entry(path: string, index: string, worktree: string): StatusEntry {
  const untracked = index === "?" && worktree === "?";
  return { path, index, worktree, staged: !untracked && index !== " " };
}

describe("gitStatusRows", () => {
  it("classe un fichier indexé", () => {
    const rows = gitStatusRows(status([entry("src/main.rs", "M", " ")]));
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      path: "src/main.rs",
      staged: true,
      worktree: false,
      stagedKind: "modified",
      stagedCode: "M",
      worktreeCode: "",
    });
  });

  it("classe un fichier non indexé", () => {
    const rows = gitStatusRows(status([entry("notes.txt", " ", "M")]));
    expect(rows[0]).toMatchObject({
      staged: false,
      worktree: true,
      worktreeKind: "modified",
      stagedCode: "",
      worktreeCode: "M",
    });
  });

  it("représente un fichier indexé puis modifié dans les deux onglets", () => {
    const rows = gitStatusRows(status([entry("memo.txt", "M", "M")]));
    expect(rows[0].staged).toBe(true);
    expect(rows[0].worktree).toBe(true);
    expect(rows[0].stagedKind).toBe("modified");
    expect(rows[0].worktreeKind).toBe("modified");
    expect(rows[0].stagedCode).toBe("M");
    expect(rows[0].worktreeCode).toBe("M");
  });

  it("classe les fichiers non suivis, ajoutés, renommés et supprimés", () => {
    const rows = gitStatusRows(
      status([
        entry("nouveau.txt", "?", "?"),
        entry("ajoute.rs", "A", " "),
        entry("renomme.rs", "R", " "),
        entry("supprime.rs", " ", "D"),
      ]),
    );
    const byPath = new Map(rows.map((row) => [row.path, row]));
    expect(byPath.get("nouveau.txt")).toMatchObject({
      staged: false,
      worktree: true,
      stagedKind: "untracked",
      worktreeKind: "untracked",
    });
    expect(byPath.get("ajoute.rs")?.stagedKind).toBe("added");
    expect(byPath.get("renomme.rs")?.stagedKind).toBe("renamed");
    expect(byPath.get("supprime.rs")?.worktreeKind).toBe("deleted");
  });

  it("détecte les conflits et les fichiers ignorés", () => {
    const conflict = gitStatusRows(status([entry("conflit.txt", "U", "U")]))[0];
    expect(conflict.stagedKind).toBe("conflict");
    expect(conflict.worktreeKind).toBe("conflict");
    expect(conflict.staged).toBe(true);

    const bothAdded = gitStatusRows(status([entry("ajoute-deux-fois.txt", "A", "A")]))[0];
    expect(bothAdded.stagedKind).toBe("conflict");
    expect(bothAdded.worktreeKind).toBe("conflict");

    const ignored = gitStatusRows(status([entry("cache.log", "!", "!")]))[0];
    expect(ignored.staged).toBe(false);
    expect(ignored.worktree).toBe(false);
    expect(ignored.stagedKind).toBe("ignored");
  });

  it("trie les lignes par chemin et filtre par onglet", () => {
    const rows = gitStatusRows(
      status([
        entry("z.txt", " ", "M"),
        entry("a.txt", "M", " "),
        entry("m.txt", "M", "M"),
      ]),
    );
    expect(rows.map((row) => row.path)).toEqual(["a.txt", "m.txt", "z.txt"]);
    expect(stagedRows(rows).map((row) => row.path)).toEqual(["a.txt", "m.txt"]);
    expect(unstagedRows(rows).map((row) => row.path)).toEqual(["m.txt", "z.txt"]);
  });

  it("convertit les codes porcelain connus", () => {
    expect(changeKindFromCode("A")).toBe("added");
    expect(changeKindFromCode("T")).toBe("modified");
    expect(changeKindFromCode("C")).toBe("added");
    expect(changeKindFromCode("U")).toBe("conflict");
    expect(changeKindFromCode("?")).toBe("untracked");
    expect(changeKindFromCode("X")).toBe("other");
  });
});
