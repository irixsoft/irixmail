import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "@/app";
import { applyTheme, initTheme, resolveTheme, systemPrefersDark, watchSystemTheme } from "@/lib/theme";
import "@/styles.css";

initTheme();
watchSystemTheme(() => applyTheme(resolveTheme("system", systemPrefersDark())));

const container = document.getElementById("root");
if (!container) throw new Error("missing #root element");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
