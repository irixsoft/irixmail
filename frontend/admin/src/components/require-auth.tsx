import { Navigate, Outlet, useLocation } from "react-router-dom";
import { ErrorState, useAuth } from "@irixmail/shared";

import { FullPageLoader } from "@/components/full-page-loader";

export function RequireAuth() {
  const { status, isAdmin, logout } = useAuth();
  const location = useLocation();

  if (status === "loading") {
    return <FullPageLoader label="Restoring session" />;
  }

  if (status === "unauthenticated") {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }

  if (!isAdmin) {
    return (
      <div className="bg-grid flex min-h-svh items-center justify-center bg-background p-6">
        <ErrorState
          title="Administrator access required"
          description="This account can use webmail, but not the admin panel."
          retryLabel="Sign out"
          onRetry={logout}
          className="max-w-md"
        />
      </div>
    );
  }

  return <Outlet />;
}
