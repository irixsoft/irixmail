import { describe, expect, it } from "vitest";

import { vcardFilename } from "./download";

describe("vcardFilename", () => {
  it("slugs a display name", () => {
    expect(vcardFilename("Ada Lovelace")).toBe("ada-lovelace.vcf");
  });

  it("collapses punctuation and repeated separators", () => {
    expect(vcardFilename("  Dr. Ada — Lovelace!! ")).toBe("dr-ada-lovelace.vcf");
  });

  it("falls back when nothing usable remains", () => {
    expect(vcardFilename("***")).toBe("contact.vcf");
    expect(vcardFilename("")).toBe("contact.vcf");
  });

  it("caps the length", () => {
    expect(vcardFilename("a".repeat(120)).length).toBeLessThanOrEqual(64);
  });
});
