import * as React from "react";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "irixmail.webmail.theme";

export function resolveTheme(mode: ThemeMode, systemDark: boolean): "light" | "dark" {
  if (mode === "system") return systemDark ? "dark" : "light";
  return mode;
}

export function applyTheme(theme: "light" | "dark") {
  document.documentElement.classList.toggle("dark", theme === "dark");
}

export function loadThemeMode(): ThemeMode {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "dark" || stored === "system" ? stored : "light";
}

export function saveThemeMode(mode: ThemeMode) {
  localStorage.setItem(STORAGE_KEY, mode);
}

export function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function initTheme(): ThemeMode {
  const mode = loadThemeMode();
  applyTheme(resolveTheme(mode, systemPrefersDark()));
  return mode;
}

export function setThemeMode(mode: ThemeMode) {
  saveThemeMode(mode);
  applyTheme(resolveTheme(mode, systemPrefersDark()));
}

export function isDarkTheme(): boolean {
  return document.documentElement.classList.contains("dark");
}

export function subscribeTheme(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
  return () => observer.disconnect();
}

export function useIsDark(): boolean {
  return React.useSyncExternalStore(subscribeTheme, isDarkTheme, () => false);
}

export function watchSystemTheme(onChange: () => void): () => void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const handler = () => {
    if (loadThemeMode() === "system") onChange();
  };
  media.addEventListener("change", handler);
  return () => media.removeEventListener("change", handler);
}
