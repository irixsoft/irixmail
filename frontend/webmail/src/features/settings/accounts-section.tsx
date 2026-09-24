import { useNavigate } from "react-router-dom";
import { Badge, Button, sameUser, useAuth } from "@irixmail/shared";
import { Plus } from "lucide-react";

import { useLogout } from "@/lib/use-logout";
import { SettingsCard } from "./section-card";

export function AccountsSection() {
  const { username, sessions, switchTo } = useAuth();
  const navigate = useNavigate();
  const signOut = useLogout();

  return (
    <SettingsCard
      title="Signed-in accounts"
      description="Every mailbox here keeps its own cache and notification settings on this device."
      bodyClassName="p-0"
      footer={
        <Button size="sm" onClick={() => void navigate("/login?add=1")}>
          <Plus className="size-4" /> Add account
        </Button>
      }
    >
      <ul className="divide-y">
        {sessions.map((session) => {
          const active = Boolean(username && sameUser(session.username, username));
          return (
            <li key={session.username} className="flex items-center justify-between gap-4 px-4 py-3">
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate font-mono text-[13px]">{session.username}</span>
                {active ? (
                  <Badge variant="muted" className="font-mono text-[10px]">
                    current
                  </Badge>
                ) : null}
              </div>
              {active ? (
                <Button variant="outline" size="sm" onClick={() => void signOut()}>
                  Sign out
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    switchTo(session.username);
                    void navigate("/");
                  }}
                >
                  Switch
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </SettingsCard>
  );
}
