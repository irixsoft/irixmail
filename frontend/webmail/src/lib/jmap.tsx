import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { JmapClient, useAuth, type JmapSession } from "@irixmail/shared";

const JmapContext = React.createContext<JmapClient | null>(null);

const MAIL = "urn:ietf:params:jmap:mail";

export function JmapProvider({ children }: { children: React.ReactNode }) {
  const { token, logout } = useAuth();
  const tokenRef = React.useRef(token);

  React.useEffect(() => {
    tokenRef.current = token;
  }, [token]);

  const client = React.useMemo(
    () => new JmapClient({ baseUrl: "", getToken: () => tokenRef.current, onUnauthorized: logout }),
    [logout],
  );

  return <JmapContext.Provider value={client}>{children}</JmapContext.Provider>;
}

export function useJmap(): JmapClient {
  const client = React.useContext(JmapContext);
  if (!client) throw new Error("useJmap must be used within a JmapProvider");
  return client;
}

export function useJmapSession() {
  const client = useJmap();
  const query = useQuery({
    queryKey: ["jmap-session"],
    queryFn: () => client.session(),
    staleTime: 5 * 60 * 1000,
  });
  const session: JmapSession | undefined = query.data;
  const accountId =
    session?.primaryAccounts?.[MAIL] ??
    (session ? Object.keys(session.accounts)[0] : undefined);
  return { session, accountId, query };
}
