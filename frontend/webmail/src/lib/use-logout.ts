import * as React from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "@irixmail/shared";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { teardownPush } from "@/pwa/web-push";

export function useLogout() {
  const { logout } = useAuth();
  const navigate = useNavigate();
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  return React.useCallback(async () => {
    await teardownPush(jmap, accountId ?? null).catch(() => undefined);
    logout();
    void navigate("/login");
  }, [jmap, accountId, logout, navigate]);
}
