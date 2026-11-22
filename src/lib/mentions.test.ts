import { describe, expect, it } from "vitest";
import {
  addChip,
  clearChips,
  extractContextPaths,
  findMentionToken,
  fitsBudget,
  insertMention,
  removeChip,
  type ContextChip,
} from "./mentions";

function chips(...paths: string[]): ContextChip[] {
  return paths.map((path) => ({ path, size: null }));
}

describe("détection du jeton @", () => {
  it("détecte une mention en fin de texte, au début ou après un espace", () => {
    expect(findMentionToken("@src")).toEqual({ start: 0, end: 4, query: "src" });
    expect(findMentionToken("regarde @src/lib/api")).toEqual({
      start: 8,
      end: 20,
      query: "src/lib/api",
    });
    expect(findMentionToken("texte @")).toEqual({ start: 6, end: 7, query: "" });
  });

  it("ignore une mention terminée, collée à un mot ou sans espace", () => {
    expect(findMentionToken("regarde @src/ ")).toBeNull();
    expect(findMentionToken("email@exemple.fr")).toBeNull();
    expect(findMentionToken("sans arobase")).toBeNull();
    expect(findMentionToken("@@double")).toBeNull();
    // Une seconde mention après un espace est bien détectée.
    expect(findMentionToken("@a @b")).toEqual({ start: 3, end: 5, query: "b" });
  });

  it("respecte un curseur au milieu du texte", () => {
    const text = "lis @api puis la suite";
    expect(findMentionToken(text, 8)).toEqual({ start: 4, end: 8, query: "api" });
    expect(findMentionToken(text, text.length)).toBeNull();
  });
});

describe("insertion d'une mention", () => {
  it("remplace le jeton par @chemin et place le curseur après l'espace", () => {
    const text = "analyse @src/li";
    const token = findMentionToken(text);
    expect(token).not.toBeNull();
    if (!token) {
      return;
    }
    const result = insertMention(text, token, "src/lib/api.ts");
    expect(result.text).toBe("analyse @src/lib/api.ts ");
    expect(result.caret).toBe(result.text.length);
    expect(result.text.slice(0, result.caret)).toBe(result.text);
  });

  it("conserve le texte qui suit le jeton", () => {
    const text = "vois @a et corrige";
    const token = findMentionToken(text, 7);
    expect(token).not.toBeNull();
    if (!token) {
      return;
    }
    const result = insertMention(text, token, "src/a.rs");
    expect(result.text).toBe("vois @src/a.rs  et corrige");
  });

  it("ne touche à rien quand le chemin est vide", () => {
    const text = "garde @";
    const token = findMentionToken(text);
    expect(token).not.toBeNull();
    if (!token) {
      return;
    }
    expect(insertMention(text, token, "  ")).toEqual({ text, caret: token.end });
  });
});

describe("chips de contexte", () => {
  it("déduplique les chemins et met à jour la taille réelle", () => {
    const initial = addChip(clearChips(), { path: "src/a.ts", size: null });
    const duplicate = addChip(initial, { path: "src/a.ts", size: 1234 });
    expect(duplicate).toHaveLength(1);
    expect(duplicate[0].size).toBe(1234);
    expect(initial[0].size).toBeNull();
    expect(addChip(duplicate, { path: "src/a.ts", size: null })[0].size).toBe(1234);
    expect(addChip(duplicate, { path: "   ", size: 1 })).toHaveLength(1);
  });

  it("extrait les chemins dans l'ordre, sans doublon ni chaîne vide", () => {
    const list = [
      { path: "src/a.ts", size: 10 },
      { path: "src/b.ts", size: null },
      { path: "src/a.ts", size: 10 },
      { path: "  ", size: null },
    ];
    expect(extractContextPaths(list)).toEqual(["src/a.ts", "src/b.ts"]);
    expect(extractContextPaths([])).toEqual([]);
  });

  it("supprime un chip sans toucher aux autres", () => {
    const list = chips("src/a.ts", "src/b.ts", "src/c.ts");
    expect(removeChip(list, "src/b.ts")).toEqual([
      { path: "src/a.ts", size: null },
      { path: "src/c.ts", size: null },
    ]);
    expect(removeChip(list, "absent.ts")).toHaveLength(3);
    expect(list).toHaveLength(3);
    expect(clearChips()).toEqual([]);
  });

  it("compare les octets au budget du contexte", () => {
    expect(fitsBudget(100, 256)).toBe(true);
    expect(fitsBudget(256, 256)).toBe(true);
    expect(fitsBudget(257, 256)).toBe(false);
    expect(fitsBudget(Number.NaN, 256)).toBe(false);
  });
});
