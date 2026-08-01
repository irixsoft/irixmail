import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import {
  Badge,
  Button,
  DataTable,
  EmptyState,
  Switch,
  toast,
  useAuth,
  type ColumnDef,
} from "@irixmail/shared";
import { Globe, Plus } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatDate } from "@/lib/format";
import type { Domain } from "@/lib/types";

function dnsBadge(status: Domain["dns_status"]) {
  switch (status.state) {
    case "verified":
      return <Badge variant="success">Verified</Badge>;
    case "failing":
      return <Badge variant="destructive">Failing</Badge>;
    default:
      return <Badge variant="muted">Unverified</Badge>;
  }
}

export function DomainsListPage() {
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["domains"],
    queryFn: ({ signal }) => client.get<{ domains: Domain[] }>("/api/domains", { signal }),
  });

  const toggle = useMutation({
    mutationFn: (input: { id: string; enabled: boolean }) =>
      client.put(`/api/domains/${input.id}`, { enabled: input.enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["domains"] }),
    onError: () => toast.error("Could not update the domain"),
  });

  const columns: ColumnDef<Domain>[] = [
    {
      accessorKey: "name",
      header: "Domain",
      cell: ({ row }) => (
        <div className="flex flex-col">
          <span className="font-medium">{row.original.name}</span>
          {row.original.aliases.length > 0 ? (
            <span className="text-xs text-muted-foreground">
              {row.original.aliases.length} alias{row.original.aliases.length === 1 ? "" : "es"}
            </span>
          ) : null}
        </div>
      ),
    },
    {
      id: "dns",
      header: "DNS",
      cell: ({ row }) => dnsBadge(row.original.dns_status),
    },
    {
      id: "enabled",
      header: "Enabled",
      cell: ({ row }) => (
        <span
          onClick={(event) => event.stopPropagation()}
          className="inline-flex"
          role="presentation"
        >
          <Switch
            checked={row.original.enabled}
            aria-label="Domain enabled"
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
        title="Domains"
        description="Mail domains served by this server"
        actions={
          <Button size="sm" onClick={() => navigate("/domains/new")}>
            <Plus className="size-4" />
            Add domain
          </Button>
        }
      />
      <DataTable
        columns={columns}
        data={query.data?.domains ?? []}
        loading={query.isLoading}
        error={query.isError ? "Could not load domains." : null}
        onRetry={() => query.refetch()}
        onRowClick={(domain) => navigate(`/domains/${domain.id}`)}
        empty={
          <EmptyState
            icon={Globe}
            title="No domains yet"
            description="Add your first mail domain to start receiving mail."
            action={
              <Button size="sm" onClick={() => navigate("/domains/new")}>
                <Plus className="size-4" />
                Add domain
              </Button>
            }
            className="rounded-none border-0 bg-transparent"
          />
        }
      />
    </div>
  );
}
