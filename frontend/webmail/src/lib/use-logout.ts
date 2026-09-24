import * as React from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "@irixmail/shared";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { teardownPush } from "@/pwa/web-push";

export function useLogout() {
  const { logout, sessions } = useAuth();
  const navigate = useNavigate();
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  return React.useCallback(async () => {
    await teardownPush(jmap, accountId ?? null).catch(() => undefined);
    const remaining = sessions.length - 1;
    logout();
    void navigate(remaining > 0 ? "/" : "/login");
  }, [jmap, accountId, sessions.length, logout, navigate]);
}
