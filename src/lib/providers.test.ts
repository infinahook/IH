import { describe, expect, it } from "vitest";
import { translate } from "./i18n";
import {
  CONNECTION_STATUSES,
  PROVIDER_KINDS,
  baseUrlError,
  connectionNoticeTone,
  connectionStatusKey,
  connectionStatusTone,
  headerList,
  isLocalHostname,
  isValidBaseUrl,
  normalizeConnectionStatus,
  parseHeaders,
  providerRequiresSecret,
  providerSlugError,
  serializeHeaders,
  sortProviders,
  splitProvidersByAvailability,
} from "./providers";

describe("validation du slug de fournisseur", () => {
  it("accepte les identifiants conformes au noyau", () => {
    expect(providerSlugError("mon-fournisseur")).toBeNull();
    expect(providerSlugError("provider_2")).toBeNull();
    expect(providerSlugError("  openai-compatible  ")).toBeNull();
    expect(providerSlugError("a1")).toBeNull();
  });

  it("refuse les identifiants vides, trop courts ou trop longs", () => {
    expect(providerSlugError("")).toBe("empty");
    expect(providerSlugError("   ")).toBe("empty");
    expect(providerSlugError("a")).toBe("length");
    expect(providerSlugError("a".repeat(33))).toBe("length");
  });

  it("refuse les majuscules, espaces, accents et ponctuation", () => {
    expect(providerSlugError("Mon-Provider")).toBe("characters");
    expect(providerSlugError("mon provider")).toBe("characters");
    expect(providerSlugError("fournisseur-é")).toBe("characters");
    expect(providerSlugError("abc!")).toBe("characters");
    expect(providerSlugError("provider.name")).toBe("characters");
  });

  it("exige un premier caractère alphanumérique minuscule", () => {
    expect(providerSlugError("-abc")).toBe("start");
    expect(providerSlugError("_abc")).toBe("start");
  });
});

describe("validation de l'URL de base", () => {
  it("accepte https vers un serveur distant", () => {
    expect(baseUrlError("https://api.example.com/v1")).toBeNull();
    expect(baseUrlError("https://api.openai.com")).toBeNull();
    expect(isValidBaseUrl("https://generativelanguage.googleapis.com")).toBe(true);
  });

  it("accepte http uniquement vers un hôte local", () => {
    const local = [
      "http://localhost:11434",
      "http://localhost",
      "http://127.0.0.1:8080/v1",
      "http://192.168.1.20:8080",
      "http://10.0.0.5",
      "http://172.16.0.1",
      "http://172.31.255.254",
      "http://[::1]:9000",
    ];
    for (const url of local) {
      expect(baseUrlError(url), url).toBeNull();
    }
    expect(isLocalHostname("localhost")).toBe(true);
    expect(isLocalHostname("127.0.0.1")).toBe(true);
    expect(isLocalHostname("::1")).toBe(true);
    expect(isLocalHostname("192.168.0.42")).toBe(true);
    expect(isLocalHostname("10.42.0.1")).toBe(true);
    expect(isLocalHostname("172.16.0.1")).toBe(true);
    expect(isLocalHostname("172.31.0.1")).toBe(true);
  });

  it("refuse http vers un serveur distant", () => {
    expect(baseUrlError("http://api.example.com")).toBe("remote_http");
    expect(baseUrlError("http://8.8.8.8")).toBe("remote_http");
    expect(baseUrlError("http://172.32.0.1")).toBe("remote_http");
    expect(isLocalHostname("172.32.0.1")).toBe(false);
    expect(isLocalHostname("example.com")).toBe(false);
  });

  it("refuse une URL vide, sans schéma ou malformée", () => {
    expect(baseUrlError("")).toBe("empty");
    expect(baseUrlError("   ")).toBe("empty");
    expect(baseUrlError("api.example.com")).toBe("scheme");
    expect(baseUrlError("ftp://example.com")).toBe("scheme");
    expect(baseUrlError("http://")).toBe("malformed");
  });
});

describe("en-têtes personnalisés", () => {
  it("analyse un en-tête par ligne et ignore les lignes vides", () => {
    const parsed = parseHeaders(
      "X-Api-Key: abc:def\n\nAuthorization: Bearer jeton\nX-Organization: mon-equipe\n",
    );
    expect(parsed.issues).toEqual([]);
    expect(parsed.headers).toEqual({
      "X-Api-Key": "abc:def",
      Authorization: "Bearer jeton",
      "X-Organization": "mon-equipe",
    });
  });

  it("sérialise les en-têtes triés et fait un aller-retour stable", () => {
    const headers = { "X-Zeta": "dernier", "X-Alpha": "premier" };
    const text = serializeHeaders(headers);
    expect(text).toBe("X-Alpha: premier\nX-Zeta: dernier");
    expect(parseHeaders(text).headers).toEqual(headers);
    expect(serializeHeaders(undefined)).toBe("");
  });

  it("refuse un nom contenant un espace", () => {
    const parsed = parseHeaders("X Api Key: secret");
    expect(parsed.headers).toEqual({});
    expect(parsed.issues).toEqual([{ line: 1, code: "invalid_name" }]);
  });

  it("signale le séparateur manquant, le nom vide et les doublons", () => {
    const parsed = parseHeaders("SansSeparateur\n: valeur\nX-Token: a\nx-token: b");
    expect(parsed.headers).toEqual({ "X-Token": "a" });
    expect(parsed.issues).toEqual([
      { line: 1, code: "missing_colon" },
      { line: 2, code: "empty_name" },
      { line: 4, code: "duplicate" },
    ]);
  });

  it("expose une liste triée nom/valeur pour l'affichage", () => {
    expect(headerList({ "X-B": "2", "X-A": "1" })).toEqual([
      { name: "X-A", value: "1" },
      { name: "X-B", value: "2" },
    ]);
    expect(headerList(null)).toEqual([]);
  });
});

describe("mapping des statuts de test de connexion", () => {
  it("traduit chaque statut du noyau en libellé français", () => {
    const expected: Record<string, string> = {
      ok: "Connexion réussie",
      auth_failed: "Authentification refusée",
      unreachable: "Endpoint injoignable",
      unsupported_protocol: "Protocole non pris en charge",
      model_unavailable: "Modèle indisponible",
      timeout: "Délai dépassé",
      error: "Erreur",
    };
    for (const status of CONNECTION_STATUSES) {
      expect(translate("fr", connectionStatusKey(status))).toBe(expected[status]);
    }
    expect(CONNECTION_STATUSES).toHaveLength(7);
  });

  it("retombe sur « erreur » pour un statut inconnu", () => {
    expect(normalizeConnectionStatus("quelque_chose")).toBe("error");
    expect(normalizeConnectionStatus(" OK ")).toBe("ok");
    expect(translate("fr", connectionStatusKey("inconnu"))).toBe("Erreur");
  });

  it("associe un ton lisible à chaque statut", () => {
    expect(connectionStatusTone("ok")).toBe("success");
    expect(connectionStatusTone("timeout")).toBe("warning");
    expect(connectionStatusTone("auth_failed")).toBe("danger");
    expect(connectionStatusTone("inconnu")).toBe("danger");
    expect(connectionNoticeTone("unreachable")).toBe("error");
    expect(connectionNoticeTone("ok")).toBe("success");
  });
});

describe("liste et tri des fournisseurs", () => {
  const integratedZeta = { id: "zeta", label: "Zeta", custom: false, enabled: true };
  const integratedBeta = { id: "beta", label: "beta", custom: false, enabled: true };
  const customAlpha = { id: "alpha", label: "Alpha", custom: true, enabled: true };
  const customDisabled = { id: "delta", label: "Delta", custom: true, enabled: false };

  it("place les intégrés d'abord puis trie alphabétiquement", () => {
    const input = [customAlpha, integratedZeta, customDisabled, integratedBeta];
    const sorted = sortProviders(input);
    expect(sorted.map((entry) => entry.id)).toEqual(["beta", "zeta", "alpha", "delta"]);
    // Le tableau source n'est pas modifié.
    expect(input.map((entry) => entry.id)).toEqual(["alpha", "zeta", "delta", "beta"]);
  });

  it("départage deux libellés identiques par identifiant", () => {
    const sorted = sortProviders([
      { id: "b", label: "Même", custom: false },
      { id: "a", label: "Même", custom: false },
    ]);
    expect(sorted.map((entry) => entry.id)).toEqual(["a", "b"]);
  });

  it("sépare les fournisseurs disponibles des désactivés", () => {
    const groups = splitProvidersByAvailability([
      integratedZeta,
      customAlpha,
      customDisabled,
    ]);
    expect(groups.enabled.map((entry) => entry.id)).toEqual(["zeta", "alpha"]);
    expect(groups.disabled.map((entry) => entry.id)).toEqual(["delta"]);
  });
});

describe("protocoles", () => {
  it("expose les quatre protocoles du noyau", () => {
    expect(PROVIDER_KINDS).toEqual([
      "openai_compatible",
      "anthropic",
      "google",
      "ollama",
    ]);
  });

  it("ne demande pas de clé pour Ollama", () => {
    expect(providerRequiresSecret("ollama")).toBe(false);
    expect(providerRequiresSecret("openai_compatible")).toBe(true);
    expect(providerRequiresSecret("anthropic")).toBe(true);
    expect(providerRequiresSecret("google")).toBe(true);
  });
});
