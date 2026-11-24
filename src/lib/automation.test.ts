import { describe, expect, it } from "vitest";
import { formatSteps, parseSteps } from "./automation";

describe("formatSteps", () => {
  it("sérialise les étapes sur plusieurs lignes", () => {
    const formatted = formatSteps([{ agent_id: "claude", prompt: "Relis" }]);
    expect(formatted).toContain('"agent_id"');
    expect(formatted.split("\n").length).toBeGreaterThan(2);
    expect(JSON.parse(formatted)).toEqual([{ agent_id: "claude", prompt: "Relis" }]);
  });
});

describe("parseSteps", () => {
  it("accepte un tableau d'étapes valides", () => {
    const { steps, error } = parseSteps(
      '[{"agent_id":"codex","prompt":"corrige"},{"agent_id":"claude","prompt":"relis"}]',
    );
    expect(error).toBeNull();
    expect(steps).toEqual([
      { agent_id: "codex", prompt: "corrige" },
      { agent_id: "claude", prompt: "relis" },
    ]);
  });

  it("refuse une saisie vide", () => {
    expect(parseSteps("   ").error).toBe("steps_required");
  });

  it("refuse un tableau vide", () => {
    expect(parseSteps("[]").error).toBe("steps_required");
  });

  it("refuse un JSON qui n'est pas un tableau", () => {
    expect(parseSteps('{"agent_id":"x"}').error).toBe("steps_required");
  });

  it("signale un JSON invalide avec son détail", () => {
    const result = parseSteps("[{");
    expect(result.error).toBe("invalid_json");
    expect(result.detail).not.toBeNull();
  });

  it("refuse une étape qui n'est pas un objet", () => {
    expect(parseSteps("[42]").error).toBe("steps_hint");
  });

  it("refuse une étape sans prompt", () => {
    expect(parseSteps('[{"agent_id":"x"}]').error).toBe("steps_hint");
  });

  it("refuse une étape avec des champs non textuels", () => {
    expect(parseSteps('[{"agent_id":"x","prompt":3}]').error).toBe("steps_hint");
  });
});
