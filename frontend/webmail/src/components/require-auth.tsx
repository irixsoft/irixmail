import { Navigate, Outlet, useLocation } from "react-router-dom";
import { useAuth } from "@irixmail/shared";

import { FullPageLoader } from "@/components/full-page-loader";

export function RequireAuth() {
  const { status } = useAuth();
  const location = useLocation();

  if (status === "loading") {
    return <FullPageLoader label="Restoring session" />;
  }
  if (status === "unauthenticated") {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  return <Outlet />;
}
