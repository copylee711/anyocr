import { describe, expect, it } from "vitest";

import { defaultSettings, mergeSettings } from "./settings";

describe("mergeSettings", () => {
  it("keeps defaults for missing or mistyped fields and ignores unknown keys", () => {
    const merged = mergeSettings(defaultSettings(), {
      engine: { device: "cpu", workers: "x" },
      doc: { dpi: 300, output: { formats: ["md"] } },
      unknown: 1,
    });
    expect(merged.engine.device).toBe("cpu");
    expect(merged.engine.workers).toBe(0);
    expect(merged.doc.dpi).toBe(300);
    expect(merged.doc.output.formats).toEqual(["md"]);
    expect(merged.doc.output.fileNameTemplate).toBe("[OCR]_%name_%date");
    expect("unknown" in merged).toBe(false);
  });
});
