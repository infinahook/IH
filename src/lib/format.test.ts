import { describe, expect, it } from "vitest";
import { countLines, formatBytes, formatDate, shortHash } from "./format";

describe("formatBytes", () => {
  it("formate les octets et les unités binaires", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1)).toBe("1 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1 KiB");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(1024 * 1024)).toBe("1 MiB");
    expect(formatBytes(1024 * 1024 * 1.5)).toBe("1.5 MiB");
    expect(formatBytes(4 * 1024 * 1024 * 1024)).toBe("4 GiB");
  });

  it("gère les valeurs invalides", () => {
    expect(formatBytes(-1)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("—");
  });
});

describe("formatDate", () => {
  it("formate une date ISO valide", () => {
    const value = formatDate("2026-11-22T12:34:56Z", "fr");
    expect(value).not.toBe("—");
    expect(value).toContain("2026");
    expect(value).toMatch(/\d{2}:\d{2}/);
  });

  it("produit un rendu différent selon la locale", () => {
    const fr = formatDate("2026-11-22T12:34:56Z", "fr");
    const en = formatDate("2026-11-22T12:34:56Z", "en");
    expect(fr).not.toBe(en);
  });

  it("renvoie un tiret pour une entrée absente ou invalide", () => {
    expect(formatDate(null, "fr")).toBe("—");
    expect(formatDate("", "en")).toBe("—");
    expect(formatDate("pas-une-date", "fr")).toBe("—");
  });
});

describe("utilitaires de texte", () => {
  it("compte les lignes", () => {
    expect(countLines("")).toBe(0);
    expect(countLines("a")).toBe(1);
    expect(countLines("a\nb\nc")).toBe(3);
  });

  it("raccourcit les identifiants longs", () => {
    expect(shortHash("abcdef", 4)).toBe("abcd");
    expect(shortHash("abc", 8)).toBe("abc");
  });
});
