import { createBrowserRouter } from "react-router-dom";

import { AdminLayout } from "@/components/admin-layout";
import { RequireAuth } from "@/components/require-auth";
import { AccountCreatePage } from "@/routes/accounts/account-create";
import { AccountDetailPage } from "@/routes/accounts/account-detail";
import { AccountsListPage } from "@/routes/accounts/accounts-list";
import { DashboardPage } from "@/routes/dashboard";
import { DomainCreatePage } from "@/routes/domains/domain-create";
import { DomainDetailPage } from "@/routes/domains/domain-detail";
import { DomainsListPage } from "@/routes/domains/domains-list";
import { IpRulesPage } from "@/routes/ip-rules";
import { LoginPage } from "@/routes/login";
import { LogsPage } from "@/routes/logs";
import { QueuePage } from "@/routes/queue";
import { SettingsPage } from "@/routes/settings";
import { TlsPage } from "@/routes/tls";

export const router = createBrowserRouter(
  [
    { path: "/login", element: <LoginPage /> },
    {
      element: <RequireAuth />,
      children: [
        {
          path: "/",
          element: <AdminLayout />,
          children: [
            { index: true, element: <DashboardPage /> },
            { path: "domains", element: <DomainsListPage /> },
            { path: "domains/new", element: <DomainCreatePage /> },
            { path: "domains/:id", element: <DomainDetailPage /> },
            { path: "accounts", element: <AccountsListPage /> },
            { path: "accounts/new", element: <AccountCreatePage /> },
            { path: "accounts/:id", element: <AccountDetailPage /> },
            { path: "queue", element: <QueuePage /> },
            { path: "ip-rules", element: <IpRulesPage /> },
            { path: "logs", element: <LogsPage /> },
            { path: "tls", element: <TlsPage /> },
            { path: "settings", element: <SettingsPage /> },
          ],
        },
      ],
    },
  ],
  { basename: "/admin" },
);
