import * as React from "react";
import { cn } from "@irixmail/shared";
import {
  Maximize2,
  Monitor,
  Moon,
  PanelBottom,
  PanelRight,
  Rows2,
  Rows3,
  Sun,
  type LucideIcon,
} from "lucide-react";

import { loadThemeMode, setThemeMode, type ThemeMode } from "@/lib/theme";
import { getLayout, saveLayout, useLayout, type ReadingPane } from "@/app/layout-store";
import { applyDensity, loadDensity, saveDensity, type Density } from "@/features/mail/density";
import { SettingsCard } from "./section-card";

const SWATCH = {
  light: { bg: "#faf7f2", panel: "#efe7da", line: "#d8cfc0", accent: "#b1813f" },
  dark: { bg: "#211e1a", panel: "#2d2924", line: "#453d33", accent: "#d9a760" },
} as const;

const THEMES: { id: ThemeMode; label: string; icon: LucideIcon }[] = [
  { id: "light", label: "Light", icon: Sun },
  { id: "dark", label: "Dark", icon: Moon },
  { id: "system", label: "System", icon: Monitor },
];

const DENSITIES: { id: Density; label: string; hint: string; icon: LucideIcon }[] = [
  { id: "cozy", label: "Cozy", hint: "Avatars and a preview line", icon: Rows3 },
  { id: "compact", label: "Compact", hint: "More messages per screen", icon: Rows2 },
];

const READING_PANE_OPTIONS: { id: ReadingPane; label: string; hint: string; icon: LucideIcon }[] = [
  { id: "right", label: "Right", hint: "Beside the message list", icon: PanelRight },
  { id: "bottom", label: "Bottom", hint: "Below the message list", icon: PanelBottom },
  { id: "off", label: "Off", hint: "Messages open full width", icon: Maximize2 },
];

function Swatch({ tone }: { tone: "light" | "dark" }) {
  const colors = SWATCH[tone];
  return (
    <div className="flex h-full gap-1 p-1.5" style={{ backgroundColor: colors.bg }}>
      <div className="w-1/4 rounded-[2px]" style={{ backgroundColor: colors.panel }} />
      <div className="flex flex-1 flex-col justify-center gap-1">
        <div className="h-1.5 w-2/3 rounded-full" style={{ backgroundColor: colors.accent }} />
        <div className="h-1.5 w-full rounded-full" style={{ backgroundColor: colors.line }} />
        <div className="h-1.5 w-4/5 rounded-full" style={{ backgroundColor: colors.line }} />
      </div>
    </div>
  );
}

function ThemeSwatch({ mode }: { mode: ThemeMode }) {
  if (mode === "system") {
    return (
      <div className="grid h-16 grid-cols-2">
        <Swatch tone="light" />
        <Swatch tone="dark" />
      </div>
    );
  }
  return (
    <div className="h-16">
      <Swatch tone={mode} />
    </div>
  );
}

export function AppearanceSection() {
  const [theme, setTheme] = React.useState<ThemeMode>(() => loadThemeMode());
  const [density, setDensity] = React.useState<Density>(() => loadDensity());
  const { readingPane } = useLayout();

  const chooseTheme = (mode: ThemeMode) => {
    setTheme(mode);
    setThemeMode(mode);
  };

  const chooseDensity = (next: Density) => {
    setDensity(next);
    saveDensity(next);
    applyDensity(next);
  };

  return (
    <div className="space-y-4">
      <SettingsCard title="Theme" description="Applies to this browser only.">
        <div className="grid gap-3 sm:grid-cols-3">
          {THEMES.map((option) => {
            const active = theme === option.id;
            const Icon = option.icon;
            return (
              <button
                key={option.id}
                type="button"
                aria-pressed={active}
                onClick={() => chooseTheme(option.id)}
                className={cn(
                  "overflow-hidden rounded-lg border text-left transition-colors",
                  active
                    ? "border-primary ring-1 ring-primary/40"
                    : "border-border hover:border-primary/40",
                )}
              >
                <ThemeSwatch mode={option.id} />
                <span
                  className={cn(
                    "flex items-center gap-2 border-t px-3 py-2 text-[13px]",
                    active ? "font-medium text-primary" : "text-muted-foreground",
                  )}
                >
                  <Icon className="size-4" />
                  {option.label}
                </span>
              </button>
            );
          })}
        </div>
      </SettingsCard>

      <SettingsCard title="Density" description="How tightly the message list is packed.">
        <div className="grid gap-3 sm:grid-cols-2">
          {DENSITIES.map((option) => {
            const active = density === option.id;
            const Icon = option.icon;
            return (
              <button
                key={option.id}
                type="button"
                aria-pressed={active}
                onClick={() => chooseDensity(option.id)}
                className={cn(
                  "flex items-start gap-3 rounded-lg border px-3 py-3 text-left transition-colors",
                  active
                    ? "border-primary bg-primary/5 ring-1 ring-primary/40"
                    : "border-border hover:border-primary/40",
                )}
              >
                <Icon className={cn("mt-0.5 size-4", active ? "text-primary" : "text-muted-foreground")} />
                <span className="space-y-0.5">
                  <span className={cn("block text-[13px] font-medium", active && "text-primary")}>
                    {option.label}
                  </span>
                  <span className="block text-xs text-muted-foreground">{option.hint}</span>
                </span>
              </button>
            );
          })}
        </div>
      </SettingsCard>

      <SettingsCard title="Reading pane" description="Where an opened message appears.">
        <div className="grid gap-3 sm:grid-cols-3">
          {READING_PANE_OPTIONS.map((option) => {
            const active = readingPane === option.id;
            const Icon = option.icon;
            return (
              <button
                key={option.id}
                type="button"
                aria-pressed={active}
                onClick={() => saveLayout({ ...getLayout(), readingPane: option.id })}
                className={cn(
                  "flex items-start gap-3 rounded-lg border px-3 py-3 text-left transition-colors",
                  active
                    ? "border-primary bg-primary/5 ring-1 ring-primary/40"
                    : "border-border hover:border-primary/40",
                )}
              >
                <Icon className={cn("mt-0.5 size-4", active ? "text-primary" : "text-muted-foreground")} />
                <span className="space-y-0.5">
                  <span className={cn("block text-[13px] font-medium", active && "text-primary")}>
                    {option.label}
                  </span>
                  <span className="block text-xs text-muted-foreground">{option.hint}</span>
                </span>
              </button>
            );
          })}
        </div>
      </SettingsCard>
    </div>
  );
}
