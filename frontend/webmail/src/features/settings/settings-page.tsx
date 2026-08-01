import * as React from "react";
import { cn } from "@irixmail/shared";
import {
  Bell,
  CalendarClock,
  LogOut,
  Palette,
  ShieldCheck,
  Tag,
  User,
  type LucideIcon,
} from "lucide-react";

import { AccountSection } from "./account-section";
import { AppearanceSection } from "./appearance-section";
import { AutoReplySection } from "./autoreply-section";
import { NotificationsSection } from "./notifications-section";
import { SecuritySection } from "./security-section";
import { TagsSection } from "./tags-section";
import { SectionHeader } from "./section-card";
import { useLogout } from "@/lib/use-logout";

interface Section {
  id: string;
  label: string;
  description: string;
  icon: LucideIcon;
  component: () => React.JSX.Element;
}

const SECTIONS = [
  {
    id: "account",
    label: "Account",
    description: "Your name, address and signature.",
    icon: User,
    component: AccountSection,
  },
  {
    id: "autoreply",
    label: "Auto-reply",
    description: "Reply automatically while you are away.",
    icon: CalendarClock,
    component: AutoReplySection,
  },
  {
    id: "appearance",
    label: "Appearance",
    description: "Theme and list density for this browser.",
    icon: Palette,
    component: AppearanceSection,
  },
  {
    id: "tags",
    label: "Tags",
    description: "Colour-coded labels for your conversations.",
    icon: Tag,
    component: TagsSection,
  },
  {
    id: "security",
    label: "Security",
    description: "Password, app passwords and two-factor.",
    icon: ShieldCheck,
    component: SecuritySection,
  },
  {
    id: "notifications",
    label: "Notifications",
    description: "How this device alerts you about new mail.",
    icon: Bell,
    component: NotificationsSection,
  },
] as const satisfies readonly Section[];

export function SettingsPage() {
  const [activeId, setActiveId] = React.useState<string>(SECTIONS[0].id);
  const active = SECTIONS.find((section) => section.id === activeId) ?? SECTIONS[0];
  const Active = active.component;
  const signOut = useLogout();

  return (
    <div className="flex h-full min-h-0 flex-col md:flex-row">
      <nav className="shrink-0 border-b bg-background md:w-56 md:border-r md:border-b-0 md:py-5">
        <p className="hidden px-5 pb-3 text-xs font-medium tracking-wider text-muted-foreground uppercase md:block">
          Settings
        </p>
        <div className="flex gap-1.5 overflow-x-auto px-3 py-2.5 md:flex-col md:gap-0.5 md:overflow-x-visible md:px-2 md:py-0">
          {SECTIONS.map((section) => {
            const isActive = section.id === active.id;
            const Icon = section.icon;
            return (
              <button
                key={section.id}
                type="button"
                aria-current={isActive ? "page" : undefined}
                onClick={() => setActiveId(section.id)}
                className={cn(
                  "flex shrink-0 items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] whitespace-nowrap transition-colors md:w-full md:rounded-md md:border-transparent md:px-2.5 md:py-2",
                  isActive
                    ? "border-primary/30 bg-primary/10 font-medium text-primary md:border-transparent md:shadow-[inset_2.5px_0_0_0_var(--primary)]"
                    : "border-border text-muted-foreground hover:bg-accent hover:text-foreground md:border-transparent",
                )}
              >
                <Icon className="size-4 shrink-0" />
                {section.label}
              </button>
            );
          })}
          <button
            type="button"
            onClick={() => {
              void signOut();
            }}
            className="flex shrink-0 items-center gap-2 rounded-full border border-border px-3 py-1.5 text-[13px] whitespace-nowrap text-destructive transition-colors hover:bg-destructive/10 md:mt-4 md:w-full md:rounded-md md:border-transparent md:px-2.5 md:py-2"
          >
            <LogOut className="size-4 shrink-0" />
            Sign out
          </button>
        </div>
      </nav>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-4 py-6 md:px-8 md:py-8">
          <SectionHeader title={active.label} description={active.description} />
          <div className="mt-5">
            <Active />
          </div>
          <p className="mt-10 text-xs text-muted-foreground">
            IRIXMAIL is free software (AGPL-3.0) ·{" "}
            <a
              href="https://github.com/irixsoft/irixmail"
              target="_blank"
              rel="noreferrer"
              className="underline underline-offset-2 hover:text-foreground"
            >
              Source code
            </a>
          </p>
        </div>
      </div>
    </div>
  );
}
