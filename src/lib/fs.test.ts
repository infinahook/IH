import { describe, expect, it } from "vitest";
import type { FileNode } from "./api";
import { buildFileTree, lineOffset } from "./fs";

function file(path: string): FileNode {
  const name = path.split("/").pop() ?? path;
  return { path, name, is_dir: false, size: 1 };
}

function dir(path: string): FileNode {
  const name = path.split("/").pop() ?? path;
  return { path, name, is_dir: true, size: 0 };
}

describe("buildFileTree", () => {
  it("construit une arborescence imbriquée", () => {
    const tree = buildFileTree([
      file("README.md"),
      dir("src"),
      file("src/main.ts"),
      dir("src/components"),
      file("src/components/Button.tsx"),
    ]);
    expect(tree.map((node) => node.node.name)).toEqual(["src", "README.md"]);
    const src = tree[0];
    expect(src.children.map((node) => node.node.name)).toEqual(["components", "main.ts"]);
    expect(src.children[0].children[0].node.path).toBe("src/components/Button.tsx");
  });

  it("place les nœuds orphelins à la racine", () => {
    const tree = buildFileTree([file("orphelin/fichier.txt")]);
    expect(tree.map((node) => node.node.path)).toEqual(["orphelin/fichier.txt"]);
  });
});

describe("lineOffset", () => {
  const content = "première\nseconde\n\ntroisième";

  it("renvoie le décalage de chaque ligne", () => {
    expect(lineOffset(content, 1)).toBe(0);
    expect(lineOffset(content, 2)).toBe(9);
    expect(lineOffset(content, 3)).toBe(17);
    expect(lineOffset(content, 4)).toBe(18);
  });

  it("borne les lignes hors du contenu", () => {
    expect(lineOffset(content, 0)).toBe(0);
    expect(lineOffset(content, 99)).toBe(content.length);
  });
});
