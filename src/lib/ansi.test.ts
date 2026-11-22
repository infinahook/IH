import { describe, expect, it } from "vitest";
import { AnsiFilter, stripAnsi } from "./ansi";

describe("stripAnsi", () => {
  it("retire les couleurs CSI", () => {
    expect(stripAnsi("\u001b[31mrouge\u001b[0m")).toBe("rouge");
    expect(stripAnsi("\u001b[1;32mgras vert\u001b[39m")).toBe("gras vert");
  });

  it("retire les séquences d'effacement et de positionnement", () => {
    expect(stripAnsi("\u001b[2J\u001b[Hcontenu")).toBe("contenu");
  });

  it("retire les titres OSC et les hyperliens", () => {
    expect(stripAnsi("\u001b]0;mon titre\u0007texte")).toBe("texte");
    expect(stripAnsi("\u001b]8;;https://example.com\u001b\\lien\u001b]8;;\u001b\\")).toBe("lien");
  });

  it("normalise les retours chariot", () => {
    expect(stripAnsi("a\r\nb")).toBe("a\nb");
    expect(stripAnsi("a\rb")).toBe("ab");
  });

  it("applique les retours arrière", () => {
    expect(stripAnsi("abc\b\bX")).toBe("aX");
    expect(stripAnsi("abc\b")).toBe("ab");
  });

  it("conserve les tabulations, sauts de ligne et accents", () => {
    expect(stripAnsi("a\tb\nc\r\n")).toBe("a\tb\nc\n");
    expect(stripAnsi("déjà vu")).toBe("déjà vu");
  });
});

describe("AnsiFilter incrémental", () => {
  it("retient une séquence incomplète entre deux morceaux", () => {
    const filter = new AnsiFilter();
    expect(filter.push("\u001b[3")).toBe("");
    expect(filter.push("1mhello \u001b[0")).toBe("hello ");
    expect(filter.push("m!")).toBe("!");
  });

  it("retient un OSC incomplet entre deux morceaux", () => {
    const filter = new AnsiFilter();
    expect(filter.push("\u001b]0;ti")).toBe("");
    expect(filter.push("tre\u0007ok")).toBe("ok");
  });

  it("vide le tampon avec flush et oublie tout avec reset", () => {
    const filter = new AnsiFilter();
    filter.push("\u001b[31");
    expect(filter.flush()).toBe("");
    expect(filter.push("\u001b[32mvert")).toBe("vert");
    expect(filter.push("\u001b[0m")).toBe("");
    filter.reset();
    expect(filter.push("neuf")).toBe("neuf");
  });

  it("ne laisse jamais passer de HTML brut", () => {
    expect(stripAnsi("\u001b[31m<b>pas du html</b>\u001b[0m")).toBe("<b>pas du html</b>");
  });
});
