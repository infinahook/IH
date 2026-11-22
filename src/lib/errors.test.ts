import { describe, expect, it } from "vitest";
import { AppError } from "./api";
import { describeError, errorIsFixable } from "./errors";
import { createTranslator } from "./i18n";

const t = createTranslator("fr");

describe("description des erreurs", () => {
  it("traduit un code connu et conserve le message brut en détail", () => {
    const described = describeError(
      t,
      new AppError("network_error", "Connexion refusée par le fournisseur", true),
    );
    expect(described.code).toBe("network_error");
    expect(described.summary).toBe("Erreur réseau");
    expect(described.detail).toBe("Connexion refusée par le fournisseur");
    expect(described.fixable).toBe(true);
  });

  it("affiche le message brut pour un code inconnu", () => {
    const described = describeError(
      t,
      new AppError("code_inexistant_42", "Détail brut du noyau", false),
    );
    expect(described.summary).toBe("Détail brut du noyau");
    expect(described.detail).toBe("");
    expect(described.fixable).toBe(false);
    expect(described.code).toBe("code_inexistant_42");
  });

  it("classe les codes corrigeables comme le noyau", () => {
    expect(errorIsFixable("provider_error")).toBe(true);
    expect(errorIsFixable("network_error")).toBe(true);
    expect(errorIsFixable("db_error")).toBe(false);
    expect(errorIsFixable("inconnu")).toBe(false);
  });
});
