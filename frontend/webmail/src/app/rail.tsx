import * as React from "react";
import { Link, useLocation } from "react-router-dom";
import {
  Avatar,
  AvatarFallback,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  cn,
  useAuth,
} from "@irixmail/shared";
import { BookUser, Calendar, LogOut, Mail, Moon, Search, Settings, Sun, type LucideIcon } from "lucide-react";

import brandIcon from "@/assets/icon.svg";

import { loadThemeMode, setThemeMode } from "@/lib/theme";
import { useLogout } from "@/lib/use-logout";

const RAIL_TILE =
  "flex size-10 items-center justify-center rounded-lg outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring";

const NON_MAIL_SECTIONS = ["calendar", "contacts", "search", "compose", "settings"];

function RailLink({
  to,
  icon: Icon,
  label,
  active,
}: {
  to: string;
  icon: LucideIcon;
  label: string;
  active: boolean;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Link
          to={to}
          aria-label={label}
          aria-current={active ? "page" : undefined}
          className={cn(
            RAIL_TILE,
            active
              ? "bg-primary/12 text-primary"
              : "text-muted-foreground hover:bg-foreground/10 hover:text-foreground",
          )}
        >
          <Icon className="size-5" />
        </Link>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

export function Rail() {
  const { username } = useAuth();
  const signOut = useLogout();
  const section = useLocation().pathname.split("/")[1] ?? "";
  const [dark, setDark] = React.useState(() => document.documentElement.classList.contains("dark"));

  const toggleTheme = () => {
    const next = loadThemeMode() === "dark" ? "light" : "dark";
    setThemeMode(next);
    setDark(next === "dark");
  };

  const initials = (username ?? "?").slice(0, 2).toUpperCase();

  return (
    <TooltipProvider delayDuration={300}>
      <aside className="flex w-14 shrink-0 flex-col items-center border-r border-sidebar-border bg-sidebar py-3">
        <img src={brandIcon} alt="IRIXMAIL" className="mb-4 size-9 rounded-xl shadow-sm" />
        <nav className="flex flex-col items-center gap-2">
          <RailLink to="/" icon={Mail} label="Mail" active={!NON_MAIL_SECTIONS.includes(section)} />
          <RailLink to="/calendar" icon={Calendar} label="Calendar" active={section === "calendar"} />
          <RailLink to="/contacts" icon={BookUser} label="Contacts" active={section === "contacts"} />
          <RailLink to="/search" icon={Search} label="Search" active={section === "search"} />
        </nav>
        <div className="mt-auto flex flex-col items-center gap-2">
          <RailLink to="/settings" icon={Settings} label="Settings" active={section === "settings"} />
          <button
            type="button"
            aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
            onClick={toggleTheme}
            className={cn(RAIL_TILE, "text-muted-foreground hover:bg-foreground/10 hover:text-foreground")}
          >
            {dark ? <Sun className="size-5" /> : <Moon className="size-5" />}
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button type="button" aria-label="Account" className="mt-1 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <Avatar className="size-8">
                  <AvatarFallback className="bg-accent font-mono text-[11px] text-accent-foreground">
                    {initials}
                  </AvatarFallback>
                </Avatar>
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent side="right" align="end" className="min-w-52">
              <DropdownMenuLabel className="font-mono text-xs font-normal text-muted-foreground">
                {username}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                onClick={() => {
                  void signOut();
                }}
              >
                <LogOut className="size-4" /> Sign out
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </aside>
    </TooltipProvider>
  );
}
