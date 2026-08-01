import { beforeEach, describe, expect, it } from "vitest";
import { applyDensity, loadDensity, saveDensity } from "./density";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute("style");
});

describe("density", () => {
  it("defaults to cozy and round-trips", () => {
    expect(loadDensity()).toBe("cozy");
    saveDensity("compact");
    expect(loadDensity()).toBe("compact");
    localStorage.setItem("irixmail.webmail.density", "weird");
    expect(loadDensity()).toBe("cozy");
  });

  it("applies css variables on the root element", () => {
    applyDensity("cozy");
    const root = document.documentElement;
    expect(root.style.getPropertyValue("--list-row-py")).toBe("10px");
    expect(root.style.getPropertyValue("--list-preview-lines")).toBe("1");
    applyDensity("compact");
    expect(root.style.getPropertyValue("--list-row-py")).toBe("6px");
    expect(root.style.getPropertyValue("--list-preview-lines")).toBe("0");
  });
});
