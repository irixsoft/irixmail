import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "react-router-dom";
import { AuthProvider, Toaster, type AuthSession } from "@irixmail/shared";

import { router } from "@/router";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 15_000 },
  },
});

async function validateAdminSession(session: AuthSession): Promise<boolean> {
  try {
    const response = await fetch("/api/auth/session", {
      headers: { Authorization: `Bearer ${session.token}` },
    });
    if (response.status === 401 || response.status === 403) return false;
    if (!response.ok) return true;
    const body = (await response.json()) as { kind?: string };
    return body.kind === "admin";
  } catch {
    return true;
  }
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider kind="admin" storageKey="irixmail.admin" baseUrl="" validate={validateAdminSession}>
        <RouterProvider router={router} />
        <Toaster position="top-right" theme="dark" richColors closeButton />
      </AuthProvider>
    </QueryClientProvider>
  );
}
