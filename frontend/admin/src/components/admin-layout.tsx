import * as React from "react";
import { NavLink, Outlet } from "react-router-dom";
import {
  Ban,
  Globe,
  LayoutDashboard,
  LogOut,
  Menu,
  ScrollText,
  Send,
  Settings,
  ShieldCheck,
  Users,
  X,
  type LucideIcon,
} from "lucide-react";
import { Button, cn, useAuth } from "@irixmail/shared";

import { Brand } from "@/components/brand";

interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
}

const NAV: NavItem[] = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/domains", label: "Domains", icon: Globe },
  { to: "/accounts", label: "Accounts", icon: Users },
  { to: "/queue", label: "Queue", icon: Send },
  { to: "/ip-rules", label: "IP rules", icon: Ban },
  { to: "/logs", label: "Logs", icon: ScrollText },
  { to: "/tls", label: "TLS", icon: ShieldCheck },
  { to: "/settings", label: "Settings", icon: Settings },
];

function navClass({ isActive }: { isActive: boolean }) {
  return cn(
    "group flex items-center gap-3 rounded-md border-l-2 px-3 py-2 text-sm font-medium transition-colors",
    isActive
      ? "border-primary bg-sidebar-accent text-sidebar-accent-foreground"
      : "border-transparent text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground",
  );
}

function NavList({ onNavigate }: { onNavigate?: () => void }) {
  return (
    <nav className="flex flex-1 flex-col gap-1 p-3">
      {NAV.map((item) => {
        const Icon = item.icon;
        return (
          <NavLink key={item.to} to={item.to} end={item.end} onClick={onNavigate} className={navClass}>
            <Icon className="size-4 shrink-0" />
            <span>{item.label}</span>
          </NavLink>
        );
      })}
    </nav>
  );
}

function AccountFooter() {
  const { username, logout } = useAuth();
  return (
    <div className="flex items-center gap-2 border-t p-3">
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-medium text-foreground">{username ?? "—"}</p>
        <p className="font-mono text-[10px] tracking-wide text-muted-foreground uppercase">Administrator</p>
      </div>
      <Button variant="ghost" size="icon" aria-label="Sign out" onClick={logout}>
        <LogOut className="size-4" />
      </Button>
    </div>
  );
}

export function AdminLayout() {
  const [mobileOpen, setMobileOpen] = React.useState(false);

  return (
    <div className="flex min-h-svh bg-background text-foreground">
      <aside className="hidden w-60 shrink-0 flex-col border-r bg-sidebar md:flex">
        <div className="flex h-14 items-center border-b px-4">
          <Brand />
        </div>
        <NavList />
        <AccountFooter />
      </aside>

      {mobileOpen ? (
        <div className="fixed inset-0 z-50 md:hidden">
          <div
            className="absolute inset-0 bg-background/70 backdrop-blur-sm"
            onClick={() => setMobileOpen(false)}
          />
          <aside className="absolute inset-y-0 left-0 flex w-64 flex-col border-r bg-sidebar shadow-xl">
            <div className="flex h-14 items-center justify-between border-b px-4">
              <Brand />
              <Button variant="ghost" size="icon" aria-label="Close menu" onClick={() => setMobileOpen(false)}>
                <X className="size-4" />
              </Button>
            </div>
            <NavList onNavigate={() => setMobileOpen(false)} />
            <AccountFooter />
          </aside>
        </div>
      ) : null}

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 items-center gap-3 border-b bg-background/80 px-4 backdrop-blur-sm md:px-6">
          <Button
            variant="ghost"
            size="icon"
            className="md:hidden"
            aria-label="Open menu"
            onClick={() => setMobileOpen(true)}
          >
            <Menu className="size-4" />
          </Button>
          <div className="md:hidden">
            <Brand />
          </div>
          <div className="ml-auto flex items-center gap-2" />
        </header>
        <main className="flex-1 overflow-x-hidden p-4 md:p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
