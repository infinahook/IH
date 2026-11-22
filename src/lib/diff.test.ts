import { describe, expect, it } from "vitest";
import {
  countDiffChanges,
  countHunkChanges,
  diffStatsLabel,
  hunkLabel,
  isDiffEmpty,
  isDiffTruncated,
  lineNumbers,
  summarizeDiff,
  type DiffHunk,
  type DiffLine,
  type LineDiff,
} from "./diff";

function line(
  kind: DiffLine["kind"],
  text: string,
  oldLine: number | null,
  newLine: number | null,
): DiffLine {
  return { kind, text, old_line: oldLine, new_line: newLine };
}

const FIRST_HUNK: DiffHunk = {
  old_start: 10,
  old_len: 5,
  new_start: 10,
  new_len: 6,
  lines: [
    line("context", "fn main() {", 10, 10),
    line("remove", "    println!(\"vieux\");", 11, null),
    line("add", "    println!(\"nouveau\");", null, 11),
    line("add", "    println!(\"encore\");", null, 12),
    line("context", "}", 12, 13),
  ],
};

const SECOND_HUNK: DiffHunk = {
  old_start: 30,
  old_len: 2,
  new_start: 31,
  new_len: 1,
  lines: [
    line("remove", "ligne disparue", 30, null),
    line("context", "ligne commune", 31, 31),
  ],
};

const DIFF: LineDiff = {
  hunks: [FIRST_HUNK, SECOND_HUNK],
  additions: 2,
  deletions: 2,
  truncated: false,
  old_lines: 31,
  new_lines: 31,
};

describe("comptage des changements", () => {
  it("compte ajouts et suppressions d'un hunk sans compter le contexte", () => {
    expect(countHunkChanges(FIRST_HUNK)).toEqual({ additions: 2, deletions: 1 });
    expect(countHunkChanges(SECOND_HUNK)).toEqual({ additions: 0, deletions: 1 });
    expect(countHunkChanges({ ...FIRST_HUNK, lines: [] })).toEqual({
      additions: 0,
      deletions: 0,
    });
  });

  it("additionne tous les hunks", () => {
    expect(countDiffChanges(DIFF)).toEqual({ additions: 2, deletions: 2 });
    expect(countDiffChanges({ hunks: [] })).toEqual({ additions: 0, deletions: 0 });
  });

  it("n'est pas influencé par les compteurs fournis par le noyau", () => {
    const inconsistent: LineDiff = { ...DIFF, additions: 99, deletions: 99 };
    expect(countDiffChanges(inconsistent)).toEqual({ additions: 2, deletions: 2 });
    expect(summarizeDiff(inconsistent).label).toBe("+2 \u22122");
  });
});

describe("libellés", () => {
  it("affiche les compteurs au format +a −d", () => {
    expect(diffStatsLabel(3, 1)).toBe("+3 \u22121");
    expect(diffStatsLabel(0, 0)).toBe("+0 \u22120");
    expect(diffStatsLabel(Number.NaN, -4)).toBe("+0 \u22120");
    expect(diffStatsLabel(2.9, 1.2)).toBe("+2 \u22121");
  });

  it("formate l'en-tête d'un hunk au format unifié", () => {
    expect(hunkLabel(FIRST_HUNK)).toBe("@@ -10,5 +10,6 @@");
    expect(hunkLabel({ old_start: 1, old_len: 0, new_start: 1, new_len: 1, lines: [] })).toBe(
      "@@ -1,0 +1,1 @@",
    );
  });

  it("expose les numéros de ligne ancien et nouveau", () => {
    expect(lineNumbers(line("context", "x", 4, 7))).toEqual({ old: 4, new: 7 });
    expect(lineNumbers(line("add", "x", null, 7))).toEqual({ old: null, new: 7 });
    expect(lineNumbers(line("remove", "x", 4, null))).toEqual({ old: 4, new: null });
  });
});

describe("troncature", () => {
  it("signale un diff tronqué et le conserve dans le résumé", () => {
    const truncated: LineDiff = { ...DIFF, truncated: true };
    expect(isDiffTruncated(truncated)).toBe(true);
    expect(isDiffTruncated(DIFF)).toBe(false);
    const summary = summarizeDiff(truncated);
    expect(summary.truncated).toBe(true);
    expect(summary.hunks).toBe(2);
    expect(summary.oldLines).toBe(31);
    expect(summary.newLines).toBe(31);
  });

  it("détecte un diff vide", () => {
    expect(isDiffEmpty({ hunks: [], additions: 0, deletions: 0 })).toBe(true);
    expect(isDiffEmpty(DIFF)).toBe(false);
  });
});
