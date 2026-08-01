import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "react-router-dom";
import { AuthProvider, Toaster } from "@irixmail/shared";

import { router } from "@/router";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 15_000 },
  },
});

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider baseUrl="">
        <RouterProvider router={router} />
        <Toaster position="top-right" theme="dark" richColors closeButton />
      </AuthProvider>
    </QueryClientProvider>
  );
}
