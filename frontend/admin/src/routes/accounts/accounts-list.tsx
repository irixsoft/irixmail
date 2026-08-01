import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import {
  Badge,
  Button,
  DataTable,
  EmptyState,
  Input,
  Switch,
  toast,
  useAuth,
  type ColumnDef,
} from "@irixmail/shared";
import { Plus, Search, Users } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatDate } from "@/lib/format";
import type { Account, Domain } from "@/lib/types";

interface AccountRow extends Account {
  address: string;
}

export function AccountsListPage() {
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [search, setSearch] = React.useState("");

  const accountsQuery = useQuery({
    queryKey: ["accounts"],
    queryFn: ({ signal }) => client.get<{ accounts: Account[] }>("/api/accounts", { signal }),
  });
  const domainsQuery = useQuery({
    queryKey: ["domains"],
    queryFn: ({ signal }) => client.get<{ domains: Domain[] }>("/api/domains", { signal }),
  });

  const toggle = useMutation({
    mutationFn: (input: { id: string; enabled: boolean }) =>
      client.put(`/api/accounts/${input.id}`, { enabled: input.enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["accounts"] }),
    onError: () => toast.error("Could not update the account"),
  });

  const domainName = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const domain of domainsQuery.data?.domains ?? []) map.set(domain.id, domain.name);
    return map;
  }, [domainsQuery.data]);

  const rows: AccountRow[] = React.useMemo(() => {
    const all = (accountsQuery.data?.accounts ?? []).map((account) => ({
      ...account,
      address: `${account.local_part}@${domainName.get(account.domain_id) ?? "?"}`,
    }));
    const term = search.trim().toLowerCase();
    if (!term) return all;
    return all.filter(
      (account) =>
        account.address.toLowerCase().includes(term) ||
        account.display_name.toLowerCase().includes(term),
    );
  }, [accountsQuery.data, domainName, search]);

  const columns: ColumnDef<AccountRow>[] = [
    {
      accessorKey: "address",
      header: "Account",
      cell: ({ row }) => (
        <div className="flex flex-col">
          <span className="font-mono text-sm">{row.original.address}</span>
          {row.original.display_name ? (
            <span className="text-xs text-muted-foreground">{row.original.display_name}</span>
          ) : null}
        </div>
      ),
    },
    {
      id: "role",
      header: "Role",
      cell: ({ row }) =>
        row.original.role === "admin" ? (
          <Badge>Admin</Badge>
        ) : (
          <Badge variant="muted">User</Badge>
        ),
    },
    {
      id: "enabled",
      header: "Enabled",
      cell: ({ row }) => (
        <span onClick={(event) => event.stopPropagation()} className="inline-flex" role="presentation">
          <Switch
            checked={row.original.enabled}
            aria-label="Account enabled"
            onCheckedChange={(checked) => toggle.mutate({ id: row.original.id, enabled: checked })}
          />
        </span>
      ),
    },
    {
      accessorKey: "created_at",
      header: "Created",
      cell: ({ row }) => (
        <span className="text-muted-foreground">{formatDate(row.original.created_at)}</span>
      ),
    },
  ];

  return (
    <div>
      <PageHeader
        title="Accounts"
        description="Mailbox owners on this server"
        actions={
          <Button size="sm" onClick={() => navigate("/accounts/new")}>
            <Plus className="size-4" />
            Add account
          </Button>
        }
      />
      <div className="relative mb-4 max-w-sm">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search accounts"
          className="pl-9"
        />
      </div>
      <DataTable
        columns={columns}
        data={rows}
        loading={accountsQuery.isLoading}
        error={accountsQuery.isError ? "Could not load accounts." : null}
        onRetry={() => accountsQuery.refetch()}
        onRowClick={(account) => navigate(`/accounts/${account.id}`)}
        empty={
          <EmptyState
            icon={Users}
            title={search ? "No matching accounts" : "No accounts yet"}
            description={search ? "Try a different search." : "Create the first mailbox to get started."}
            className="rounded-none border-0 bg-transparent"
          />
        }
      />
    </div>
  );
}
