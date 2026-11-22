import { describe, expect, it } from "vitest";
import {
  catalogs,
  createTranslator,
  flattenKeys,
  normalizeLocale,
  translate,
  translateFrom,
  type Catalogs,
} from "./i18n";

const synthetic: Catalogs = {
  fr: {
    app: { name: "IH", greeting: "Bonjour {name}, vous avez {count} tâche(s)." },
    only_fr: "Présent en français seulement",
  },
  en: {
    app: { name: "IH", greeting: "Hello {name}, you have {count} task(s)." },
  },
};

describe("traduction", () => {
  it("traduit une clé pointée dans la langue demandée", () => {
    expect(translateFrom(synthetic, "en", "app.name")).toBe("IH");
    expect(translateFrom(synthetic, "en", "app.greeting", { name: "Ada", count: 2 })).toBe(
      "Hello Ada, you have 2 task(s).",
    );
  });

  it("retombe sur le français quand la clé manque dans la langue active", () => {
    expect(translateFrom(synthetic, "en", "only_fr")).toBe("Présent en français seulement");
    expect(translateFrom(synthetic, "fr", "only_fr")).toBe("Présent en français seulement");
  });

  it("renvoie la clé brute quand elle n'existe nulle part", () => {
    expect(translateFrom(synthetic, "en", "missing.key")).toBe("missing.key");
  });

  it("substitue toutes les occurrences des variables", () => {
    const catalogsWithRepeat: Catalogs = {
      fr: { msg: "{x} et encore {x}" },
      en: { msg: "{x} and again {x}" },
    };
    expect(translateFrom(catalogsWithRepeat, "fr", "msg", { x: "A" })).toBe("A et encore A");
  });

  it("utilise les catalogues embarqués et garde la clé brute pour l'inconnu", () => {
    expect(translate("en", "nav.projects")).toBe("Projects");
    expect(translate("fr", "nav.projects")).toBe("Projets");
    expect(translate("fr", "clé.inexistante")).toBe("clé.inexistante");
    expect(translate("fr", "common.agent_count", { count: 3 })).toBe("3 agents actifs");
  });

  it("normalise les variantes de langue", () => {
    expect(normalizeLocale("en-US")).toBe("en");
    expect(normalizeLocale("fr_FR")).toBe("fr");
    expect(normalizeLocale(null)).toBe("fr");
    expect(normalizeLocale("de")).toBe("fr");
  });

  it("expose une fonction t liée à une locale", () => {
    const t = createTranslator("en");
    expect(t("common.save")).toBe("Save");
  });

  it("maintient des catalogues FR et EN strictement parallèles", () => {
    const frKeys = flattenKeys(catalogs.fr);
    const enKeys = flattenKeys(catalogs.en);
    expect(frKeys.length).toBeGreaterThan(700);
    expect(enKeys).toEqual(frKeys);
  });
});
