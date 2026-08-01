import { beforeEach, describe, expect, it } from "vitest";
import { applyTheme, isDarkTheme, loadThemeMode, resolveTheme, saveThemeMode, subscribeTheme } from "./theme";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove("dark");
});

describe("resolveTheme", () => {
  it("passes light and dark through", () => {
    expect(resolveTheme("light", false)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("follows the system preference for system", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
});

describe("applyTheme", () => {
  it("toggles the dark class on the root element", () => {
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

describe("subscribeTheme", () => {
  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  it("reports the current theme and notifies until unsubscribed", async () => {
    let calls = 0;
    expect(isDarkTheme()).toBe(false);
    const unsubscribe = subscribeTheme(() => {
      calls += 1;
    });
    applyTheme("dark");
    await flush();
    expect(calls).toBe(1);
    expect(isDarkTheme()).toBe(true);
    unsubscribe();
    applyTheme("light");
    await flush();
    expect(calls).toBe(1);
  });
});

describe("persistence", () => {
  it("defaults to light when nothing is stored", () => {
    expect(loadThemeMode()).toBe("light");
  });

  it("round-trips a stored mode and rejects junk", () => {
    saveThemeMode("dark");
    expect(loadThemeMode()).toBe("dark");
    localStorage.setItem("irixmail.webmail.theme", "purple");
    expect(loadThemeMode()).toBe("light");
  });
});
