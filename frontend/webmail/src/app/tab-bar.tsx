import { NavLink, useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { cn } from "@irixmail/shared";
import {
  BookUser,
  Calendar,
  CalendarPlus,
  Mail,
  Settings,
  SquarePen,
  UserPlus,
  type LucideIcon,
} from "lucide-react";

function Tab({ to, icon: Icon, label }: { to: string; icon: LucideIcon; label: string }) {
  return (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        cn(
          "flex flex-1 flex-col items-center gap-0.5 py-1.5 text-[10px]",
          "focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60",
          isActive ? "font-semibold text-primary" : "text-muted-foreground",
        )
      }
    >
      <Icon className="size-5" />
      {label}
    </NavLink>
  );
}

export function TabBar() {
  const navigate = useNavigate();
  const location = useLocation();
  const [, setParams] = useSearchParams();

  const onCalendar = location.pathname.startsWith("/calendar");
  const onContacts = location.pathname.startsWith("/contacts");
  const ActionIcon = onCalendar ? CalendarPlus : onContacts ? UserPlus : SquarePen;
  const actionLabel = onCalendar ? "New event" : onContacts ? "New contact" : "Compose";

  const act = () => {
    if (onCalendar) {
      setParams((current) => {
        current.set("create", "1");
        return current;
      });
      return;
    }
    navigate(onContacts ? "/contacts/new" : "/compose");
  };

  return (
    <>
      <nav className="flex shrink-0 items-center border-t bg-card pb-[env(safe-area-inset-bottom)]">
        <Tab to="/" icon={Mail} label="Mail" />
        <Tab to="/calendar" icon={Calendar} label="Calendar" />
        <Tab to="/contacts" icon={BookUser} label="Contacts" />
        <Tab to="/settings" icon={Settings} label="Settings" />
      </nav>

      <button
        type="button"
        aria-label={actionLabel}
        onClick={act}
        className="fixed bottom-20 right-4 z-20 flex size-13 items-center justify-center rounded-2xl bg-gradient-to-br from-primary to-primary/75 text-primary-foreground shadow-lg shadow-primary/25 transition-transform active:scale-95 focus-visible:outline-none focus-visible:ring-[2px] focus-visible:ring-ring/60"
      >
        <ActionIcon className="size-5" />
      </button>
    </>
  );
}
