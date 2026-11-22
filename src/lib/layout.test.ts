import { describe, expect, it } from "vitest";
import {
  DEFAULT_LAYOUT,
  MAX_EXPLORER_WIDTH,
  MIN_BOTTOM_HEIGHT,
  MIN_CONVERSATION_WIDTH,
  MIN_EXPLORER_WIDTH,
  clampBottomHeight,
  clampConversationWidth,
  clampExplorerWidth,
  clampLayout,
  maxBottomHeight,
  maxConversationWidth,
  parseLayout,
  serializeLayout,
  type WorkspaceLayout,
} from "./layout";

const BOUNDS = { width: 1600, height: 900 };

function layout(overrides: Partial<WorkspaceLayout> = {}): WorkspaceLayout {
  return { ...DEFAULT_LAYOUT, ...overrides };
}

describe("clamping des séparateurs", () => {
  it("borne la largeur de la conversation entre le minimum et la place de l'éditeur", () => {
    expect(clampConversationWidth(10, BOUNDS)).toBe(MIN_CONVERSATION_WIDTH);
    expect(clampConversationWidth(99999, BOUNDS)).toBe(maxConversationWidth(BOUNDS));
    expect(clampConversationWidth(720, BOUNDS)).toBe(720);
    expect(clampConversationWidth(Number.NaN, BOUNDS)).toBe(DEFAULT_LAYOUT.conversation_width);
    // Zone étroite : le minimum reste prioritaire, jamais de largeur négative.
    expect(clampConversationWidth(800, { width: 640, height: 600 })).toBe(
      MIN_CONVERSATION_WIDTH,
    );
  });

  it("borne la largeur de l'explorateur", () => {
    expect(clampExplorerWidth(0, BOUNDS)).toBe(MIN_EXPLORER_WIDTH);
    expect(clampExplorerWidth(9999, BOUNDS)).toBe(MAX_EXPLORER_WIDTH);
    expect(clampExplorerWidth(300, BOUNDS)).toBe(300);
  });

  it("borne la hauteur du panneau bas sans écraser le workspace", () => {
    expect(clampBottomHeight(0, BOUNDS)).toBe(MIN_BOTTOM_HEIGHT);
    expect(clampBottomHeight(9999, BOUNDS)).toBe(maxBottomHeight(BOUNDS));
    expect(maxBottomHeight(BOUNDS)).toBeLessThan(BOUNDS.height);
    expect(clampBottomHeight(280, BOUNDS)).toBe(280);
  });

  it("normalise une disposition complète : onglets dédupliqués et actif cohérent", () => {
    const clamped = clampLayout(
      layout({
        conversation_width: -40,
        explorer_width: 9000,
        bottom_height: 12,
        bottom_tab: "inconnu" as never,
        explorer_visible: false,
        tabs: ["a.ts", "a.ts", " ", "b.ts"],
        active_tab: "disparu.ts",
      }),
      BOUNDS,
    );
    expect(clamped.conversation_width).toBe(MIN_CONVERSATION_WIDTH);
    expect(clamped.explorer_width).toBe(MAX_EXPLORER_WIDTH);
    expect(clamped.bottom_height).toBe(MIN_BOTTOM_HEIGHT);
    expect(clamped.bottom_tab).toBe("terminal");
    expect(clamped.explorer_visible).toBe(false);
    expect(clamped.tabs).toEqual(["a.ts", "b.ts"]);
    expect(clamped.active_tab).toBeNull();
  });

  it("conserve l'onglet actif quand il fait partie des onglets ouverts", () => {
    const clamped = clampLayout(
      layout({ tabs: ["src/a.ts", "src/b.ts"], active_tab: "src/b.ts" }),
      BOUNDS,
    );
    expect(clamped.active_tab).toBe("src/b.ts");
  });
});

describe("sérialisation de la disposition", () => {
  it("fait un aller-retour stable", () => {
    const source = layout({
      project_id: "p-1",
      conversation_width: 700,
      explorer_width: 320,
      explorer_visible: false,
      bottom_height: 300,
      bottom_open: true,
      bottom_tab: "git",
      tabs: ["src/App.tsx", "src/lib/api.ts"],
      active_tab: "src/lib/api.ts",
    });
    const parsed = parseLayout(serializeLayout(source));
    expect(parsed).toEqual({ ...source, conversation_width: 700, bottom_height: 300 });
    // Sérialiser le résultat ne change plus rien.
    expect(serializeLayout(parsed)).toBe(serializeLayout(source));
  });

  it("sérialise des largeurs entières et un JSON lisible par le noyau", () => {
    const raw = serializeLayout(layout({ conversation_width: 640.6 }));
    const payload = JSON.parse(raw) as Record<string, unknown>;
    expect(payload.version).toBe(1);
    expect(payload.conversation_width).toBe(641);
    expect(payload.explorer_visible).toBe(true);
    expect(typeof payload).toBe("object");
  });
});

describe("valeurs corrompues", () => {
  it("retombe sur la disposition par défaut si la charge est absente ou illisible", () => {
    expect(parseLayout(null)).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("")).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("{pas du json")).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("[]")).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("42")).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout(JSON.stringify({ version: 99, conversation_width: 500 }))).toEqual(
      DEFAULT_LAYOUT,
    );
  });

  it("remplace chaque valeur aberrante par sa valeur par défaut", () => {
    const parsed = parseLayout(
      JSON.stringify({
        version: 1,
        conversation_width: -10,
        explorer_width: "large",
        bottom_height: 1e9,
        explorer_visible: "oui",
        bottom_open: "non",
        bottom_tab: "n'importe quoi",
        tabs: ["ok.ts", 42, null, "ok.ts"],
        active_tab: "absent.ts",
      }),
    );
    expect(parsed.conversation_width).toBe(DEFAULT_LAYOUT.conversation_width);
    expect(parsed.explorer_width).toBe(DEFAULT_LAYOUT.explorer_width);
    expect(parsed.bottom_height).toBe(DEFAULT_LAYOUT.bottom_height);
    expect(parsed.explorer_visible).toBe(true);
    expect(parsed.bottom_open).toBe(false);
    expect(parsed.bottom_tab).toBe("terminal");
    expect(parsed.tabs).toEqual(["ok.ts"]);
    expect(parsed.active_tab).toBeNull();
  });

  it("accepte les clés camelCase d'une ancienne disposition", () => {
    const parsed = parseLayout(
      JSON.stringify({
        conversationWidth: 700,
        explorerWidth: 300,
        bottomHeight: 250,
        bottomTab: "problems",
        tabs: ["a.rs"],
        activeTab: "a.rs",
      }),
    );
    expect(parsed.conversation_width).toBe(700);
    expect(parsed.explorer_width).toBe(300);
    expect(parsed.bottom_height).toBe(250);
    expect(parsed.bottom_tab).toBe("problems");
    expect(parsed.active_tab).toBe("a.rs");
  });
});
