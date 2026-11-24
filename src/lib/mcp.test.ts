import { describe, expect, it } from "vitest";
import { riskAtLeast, riskTone, schemaFields, statusTone, summarizeResult } from "./mcp";

describe("schemaFields", () => {
  it("extrait nom, type et caractère requis des propriétés", () => {
    const fields = schemaFields({
      type: "object",
      properties: { path: { type: "string" }, count: { type: "integer" } },
      required: ["path"],
    });
    expect(fields).toEqual([
      { name: "path", type: "string", required: true },
      { name: "count", type: "integer", required: false },
    ]);
  });

  it("tolère un schéma absent ou malformé", () => {
    expect(schemaFields(null)).toEqual([]);
    expect(schemaFields("oops")).toEqual([]);
    expect(schemaFields({})).toEqual([]);
  });
});

describe("riskAtLeast", () => {
  it("compare les niveaux de risque", () => {
    expect(riskAtLeast("critical", "high")).toBe(true);
    expect(riskAtLeast("low", "medium")).toBe(false);
    expect(riskAtLeast("medium", "medium")).toBe(true);
  });
});

describe("tones", () => {
  it("associe les risques et statuts aux couleurs de badge", () => {
    expect(riskTone("low")).toBe("success");
    expect(riskTone("critical")).toBe("danger");
    expect(riskTone("inconnu")).toBe("neutral");
    expect(statusTone("error", false)).toBe("danger");
    expect(statusTone("connected", false)).toBe("success");
    expect(statusTone("disconnected", true)).toBe("success");
  });
});

describe("summarizeResult", () => {
  it("tronque les résultats trop longs", () => {
    expect(summarizeResult("abc", 10)).toBe("abc");
    const long = "x".repeat(20);
    expect(summarizeResult(long, 10)).toBe(`${"x".repeat(10)}…`);
  });
});
