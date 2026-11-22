import { describe, expect, it } from "vitest";
import {
  activateTabState,
  closeTabState,
  createTab,
  fileTitle,
  isActiveTabDirty,
  isTabDirty,
  markTabSaved,
  openTabState,
  planCloseTab,
  reloadTab,
  restoreTabs,
  updateTabContent,
  type EditorTab,
} from "./tabs";

function tab(path: string, content = "contenu"): EditorTab {
  return createTab({ path, content, sha256: `sha-${path}`, size: content.length });
}

function tabState(...paths: string[]): { tabs: EditorTab[]; activePath: string | null } {
  return { tabs: paths.map((path) => tab(path)), activePath: paths[0] ?? null };
}

describe("ouverture et activation des onglets", () => {
  it("ouvre un onglet et l'active", () => {
    const state = openTabState([], null, tab("src/a.ts"));
    expect(state.tabs.map((entry) => entry.path)).toEqual(["src/a.ts"]);
    expect(state.activePath).toBe("src/a.ts");
    expect(state.tabs[0].title).toBe("a.ts");
  });

  it("active l'onglet existant au lieu d'en créer un doublon", () => {
    const initial = tabState("src/a.ts", "src/b.ts");
    const next = openTabState(initial.tabs, initial.activePath, tab("src/b.ts", "autre buffer"));
    expect(next.tabs).toHaveLength(2);
    expect(next.activePath).toBe("src/b.ts");
    // Le buffer existant n'est pas remplacé par le nouveau contenu.
    expect(next.tabs[1].content).toBe("contenu");
    expect(next.tabs[1]).toEqual(initial.tabs[1]);
    expect(next.tabs).not.toBe(initial.tabs);
  });

  it("ignore l'activation d'un chemin inconnu", () => {
    const initial = tabState("src/a.ts");
    expect(activateTabState(initial.tabs, initial.activePath, "absent.ts").activePath).toBe(
      "src/a.ts",
    );
    expect(activateTabState(initial.tabs, initial.activePath, "src/a.ts").activePath).toBe(
      "src/a.ts",
    );
  });
});

describe("état modifié", () => {
  it("détecte un buffer différent du disque", () => {
    const [first] = tabState("src/a.ts").tabs;
    expect(isTabDirty(first)).toBe(false);
    const dirty = updateTabContent([first], "src/a.ts", "modifié")[0];
    expect(isTabDirty(dirty)).toBe(true);
    expect(isActiveTabDirty([dirty], "src/a.ts")).toBe(true);
    expect(isActiveTabDirty([dirty], null)).toBe(false);
  });

  it("remet l'onglet propre après sauvegarde ou rechargement", () => {
    const [first] = tabState("src/a.ts").tabs;
    const dirty = updateTabContent([first], "src/a.ts", "v2")[0];
    const saved = markTabSaved([dirty], "src/a.ts", {
      content: "v2",
      sha256: "sha-v2",
      size: 2,
    })[0];
    expect(isTabDirty(saved)).toBe(false);
    expect(saved.sha256).toBe("sha-v2");
    expect(saved.size).toBe(2);

    const reloaded = reloadTab([dirty], "src/a.ts", {
      content: "disque",
      sha256: "sha-disque",
      size: 6,
    })[0];
    expect(reloaded.content).toBe("disque");
    expect(reloaded.savedContent).toBe("disque");
    expect(isTabDirty(reloaded)).toBe(false);
  });
});

describe("fermeture des onglets", () => {
  it("ferme l'onglet actif et active le voisin de droite", () => {
    const initial = tabState("src/a.ts", "src/b.ts", "src/c.ts");
    const next = closeTabState(initial.tabs, "src/b.ts", "src/b.ts");
    expect(next.tabs.map((entry) => entry.path)).toEqual(["src/a.ts", "src/c.ts"]);
    expect(next.activePath).toBe("src/c.ts");
  });

  it("active le dernier onglet quand on ferme le dernier actif", () => {
    const initial = tabState("src/a.ts", "src/b.ts");
    const next = closeTabState(initial.tabs, "src/b.ts", "src/b.ts");
    expect(next.activePath).toBe("src/a.ts");
    expect(closeTabState(initial.tabs, "src/a.ts", "src/a.ts").activePath).toBe("src/b.ts");
    const empty = closeTabState(initial.tabs, "src/a.ts", "src/a.ts");
    expect(closeTabState(empty.tabs, empty.activePath, "src/b.ts")).toEqual({
      tabs: [],
      activePath: null,
    });
  });

  it("conserve l'actif quand on ferme un autre onglet", () => {
    const initial = tabState("src/a.ts", "src/b.ts");
    const next = closeTabState(initial.tabs, "src/a.ts", "src/b.ts");
    expect(next.activePath).toBe("src/a.ts");
  });

  it("exige une confirmation uniquement si l'onglet est modifié", () => {
    const [clean] = tabState("src/a.ts").tabs;
    const dirty = updateTabContent([clean], "src/a.ts", "brouillon")[0];
    expect(planCloseTab([clean], "src/a.ts")).toEqual({
      path: "src/a.ts",
      requiresConfirmation: false,
    });
    expect(planCloseTab([dirty], "src/a.ts").requiresConfirmation).toBe(true);
    expect(planCloseTab([dirty], "absent.ts").requiresConfirmation).toBe(false);
  });
});

describe("restauration des onglets", () => {
  it("relit les fichiers dans l'ordre persisté et ignore les disparus", () => {
    const restored = restoreTabs(
      ["src/a.ts", "src/disparu.ts", "src/b.ts"],
      [
        { path: "src/b.ts", content: "B", sha256: "sha-b", size: 1 },
        { path: "src/a.ts", content: "A", sha256: "sha-a", size: 1 },
      ],
      "src/b.ts",
    );
    expect(restored.tabs.map((entry) => entry.path)).toEqual(["src/a.ts", "src/b.ts"]);
    expect(restored.activePath).toBe("src/b.ts");
  });

  it("retombe sur le premier onglet restauré quand l'actif a disparu", () => {
    const restored = restoreTabs(
      ["src/a.ts", "src/disparu.ts"],
      [{ path: "src/a.ts", content: "A", sha256: "sha-a", size: 1 }],
      "src/disparu.ts",
    );
    expect(restored.tabs).toHaveLength(1);
    expect(restored.activePath).toBe("src/a.ts");
  });

  it("retombe sur aucun actif quand plus aucun onglet ne subsiste", () => {
    const restored = restoreTabs(
      ["src/disparu.ts"],
      [],
      "src/disparu.ts",
    );
    expect(restored.tabs).toEqual([]);
    expect(restored.activePath).toBeNull();
  });

  it("restaure un fichier marqué en lecture seule", () => {
    const restored = restoreTabs(
      ["notes.md"],
      [{ path: "notes.md", content: "x", sha256: "s", size: 1, readOnly: true }],
      "notes.md",
    );
    expect(restored.tabs[0].readOnly).toBe(true);
    expect(restored.activePath).toBe("notes.md");
  });

  it("ne restaure rien sans chemin persisté", () => {
    expect(restoreTabs([], [{ path: "a", content: "", sha256: "s", size: 0 }])).toEqual({
      tabs: [],
      activePath: null,
    });
  });
});

describe("titre de fichier", () => {
  it("extrait le dernier segment du chemin", () => {
    expect(fileTitle("src/lib/api.ts")).toBe("api.ts");
    expect(fileTitle("README.md")).toBe("README.md");
    expect(fileTitle("dossier/")).toBe("dossier");
    expect(fileTitle("")).toBe("");
  });
});
