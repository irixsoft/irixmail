export type Density = "cozy" | "compact";

const STORAGE_KEY = "irixmail.webmail.density";

const VARS: Record<Density, Record<string, string>> = {
  cozy: {
    "--list-row-py": "10px",
    "--list-row-gap": "10px",
    "--list-preview-lines": "1",
    "--list-avatar-size": "32px",
  },
  compact: {
    "--list-row-py": "6px",
    "--list-row-gap": "8px",
    "--list-preview-lines": "0",
    "--list-avatar-size": "0px",
  },
};

export function loadDensity(): Density {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "compact" ? "compact" : "cozy";
}

export function saveDensity(density: Density) {
  localStorage.setItem(STORAGE_KEY, density);
}

export function applyDensity(density: Density) {
  const root = document.documentElement;
  for (const [name, value] of Object.entries(VARS[density])) {
    root.style.setProperty(name, value);
  }
}

export function initDensity(): Density {
  const density = loadDensity();
  applyDensity(density);
  return density;
}
