import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "react-router-dom";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ConfirmDialog,
  ErrorState,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  toast,
  useAuth,
} from "@irixmail/shared";
import { ArrowLeft, Trash2 } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { DomainAliasesTab } from "@/routes/domains/domain-aliases-tab";
import { DomainDkimTab } from "@/routes/domains/domain-dkim-tab";
import { DomainDnsTab } from "@/routes/domains/domain-dns-tab";
import { formatDateTime } from "@/lib/format";
import type { Account, Domain } from "@/lib/types";

function dnsBadge(state: Domain["dns_status"]["state"]) {
  if (state === "verified") return <Badge variant="success">Verified</Badge>;
  if (state === "failing") return <Badge variant="destructive">Failing</Badge>;
  return <Badge variant="muted">Unverified</Badge>;
}

function DetailSkeleton() {
  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-9 w-full max-w-md" />
      <Skeleton className="h-40 w-full" />
    </div>
  );
}

export function DomainDetailPage() {
  const { id } = useParams();
  const domainId = id ?? "";
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirmDelete, setConfirmDelete] = React.useState(false);

  const query = useQuery({
    queryKey: ["domain", domainId],
    queryFn: ({ signal }) => client.get<{ domain: Domain }>(`/api/domains/${domainId}`, { signal }),
    enabled: domainId !== "",
  });
  const accountsQuery = useQuery({
    queryKey: ["accounts"],
    queryFn: ({ signal }) => client.get<{ accounts: Account[] }>("/api/accounts", { signal }),
  });

  const update = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      client.put<{ domain: Domain }>(`/api/domains/${domainId}`, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["domain", domainId] });
      void queryClient.invalidateQueries({ queryKey: ["domains"] });
    },
    onError: () => toast.error("Could not update the domain"),
  });

  const remove = useMutation({
    mutationFn: () => client.delete(`/api/domains/${domainId}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["domains"] });
      toast.success("Domain deleted");
      navigate("/domains");
    },
    onError: () => toast.error("Could not delete the domain"),
  });

  if (query.isError) {
    return (
      <div className="mx-auto max-w-3xl">
        <ErrorState description="Could not load this domain." onRetry={() => query.refetch()} />
      </div>
    );
  }
  if (query.isLoading || !query.data) return <DetailSkeleton />;

  const domain = query.data.domain;
  const domainAccounts = (accountsQuery.data?.accounts ?? []).filter(
    (account) => account.domain_id === domain.id,
  );

  return (
    <div className="mx-auto max-w-3xl">
      <Button variant="ghost" size="sm" className="mb-2" onClick={() => navigate("/domains")}>
        <ArrowLeft className="size-4" />
        Domains
      </Button>
      <PageHeader
        title={<span className="font-mono">{domain.name}</span>}
        description={`Added ${formatDateTime(domain.created_at)}`}
        actions={dnsBadge(domain.dns_status.state)}
      />

      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="dns">DNS</TabsTrigger>
          <TabsTrigger value="dkim">DKIM</TabsTrigger>
          <TabsTrigger value="aliases">Aliases</TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-4 pt-4">
          <Card>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <p className="text-sm font-medium">Enabled</p>
                  <p className="text-xs text-muted-foreground">Accept mail for this domain</p>
                </div>
                <Switch
                  checked={domain.enabled}
                  aria-label="Domain enabled"
                  onCheckedChange={(checked) => update.mutate({ enabled: checked })}
                />
              </div>
              <div className="flex items-center justify-between border-t pt-4 text-sm">
                <span className="text-muted-foreground">DNS status</span>
                {dnsBadge(domain.dns_status.state)}
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardContent className="space-y-3">
              <div>
                <p className="text-sm font-medium">Catch-all account</p>
                <p className="text-xs text-muted-foreground">
                  Deliver mail for unknown addresses to this account.
                </p>
              </div>
              <Select
                value={domain.catch_all_account_id || undefined}
                onValueChange={(value) => update.mutate({ catchAllAccountId: value })}
                disabled={domainAccounts.length === 0}
              >
                <SelectTrigger className="w-full">
                  <SelectValue
                    placeholder={
                      domainAccounts.length ? "Select an account" : "No accounts in this domain"
                    }
                  />
                </SelectTrigger>
                <SelectContent>
                  {domainAccounts.map((account) => (
                    <SelectItem key={account.id} value={account.id}>
                      {account.local_part}@{domain.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </CardContent>
          </Card>

          <Card className="border-destructive/30">
            <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <p className="text-sm font-medium">Delete domain</p>
                <p className="text-xs text-muted-foreground">
                  Removes the domain, its DNS configuration, and stored DKIM keys.
                </p>
              </div>
              <Button variant="destructive" onClick={() => setConfirmDelete(true)}>
                <Trash2 className="size-4" />
                Delete
              </Button>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="dns" className="pt-4">
          <DomainDnsTab domainId={domain.id} />
        </TabsContent>
        <TabsContent value="dkim" className="pt-4">
          <DomainDkimTab domainId={domain.id} />
        </TabsContent>
        <TabsContent value="aliases" className="pt-4">
          <DomainAliasesTab domain={domain} />
        </TabsContent>
      </Tabs>

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title="Delete domain"
        description={`This permanently removes ${domain.name} and its configuration.`}
        confirmLabel="Delete domain"
        destructive
        closeOnConfirm={false}
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}
