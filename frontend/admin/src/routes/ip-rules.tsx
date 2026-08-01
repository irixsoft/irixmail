import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ApiError,
  Badge,
  Button,
  ConfirmDialog,
  DataTable,
  EmptyState,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  toast,
  useAuth,
  type ColumnDef,
} from "@irixmail/shared";
import { Ban, Plus, Trash2 } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import type { IpRule, IpRuleAction } from "@/lib/types";

function actionBadge(action: IpRuleAction) {
  return action === "allow" ? (
    <Badge variant="success">Allow</Badge>
  ) : (
    <Badge variant="destructive">Block</Badge>
  );
}

export function IpRulesPage() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [cidr, setCidr] = React.useState("");
  const [action, setAction] = React.useState<IpRuleAction>("block");
  const [pendingDelete, setPendingDelete] = React.useState<IpRule | null>(null);

  const query = useQuery({
    queryKey: ["ip-rules"],
    queryFn: ({ signal }) => client.get<{ rules: IpRule[] }>("/api/ip-rules", { signal }),
  });

  const create = useMutation({
    mutationFn: (input: { cidr: string; action: IpRuleAction }) =>
      client.post<IpRule>("/api/ip-rules", input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["ip-rules"] });
      setCidr("");
      toast.success("Rule added");
    },
    onError: (error) =>
      toast.error(error instanceof ApiError ? error.message : "Could not add the rule"),
  });

  const remove = useMutation({
    mutationFn: (id: string) => client.delete(`/api/ip-rules/${encodeURIComponent(id)}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["ip-rules"] });
      setPendingDelete(null);
      toast.success("Rule removed");
    },
    onError: () => toast.error("Could not remove the rule"),
  });

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = cidr.trim();
    if (!trimmed) {
      toast.error("Enter a network in CIDR form, like 203.0.113.0/24");
      return;
    }
    create.mutate({ cidr: trimmed, action });
  };

  const columns: ColumnDef<IpRule>[] = [
    {
      accessorKey: "cidr",
      header: "Network",
      cell: ({ row }) => <span className="font-mono">{row.original.cidr}</span>,
    },
    {
      id: "action",
      header: "Action",
      cell: ({ row }) => actionBadge(row.original.action),
    },
    {
      id: "remove",
      header: "",
      cell: ({ row }) => (
        <div className="flex justify-end">
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Remove ${row.original.cidr}`}
            onClick={() => setPendingDelete(row.original)}
          >
            <Trash2 className="size-4 text-destructive" />
          </Button>
        </div>
      ),
    },
  ];

  return (
    <div>
      <PageHeader
        title="IP rules"
        description="Allow or block client networks before any session starts. Allow rules override block rules."
      />

      <form onSubmit={submit} className="mb-4 flex flex-col gap-2 sm:flex-row">
        <Input
          value={cidr}
          onChange={(event) => setCidr(event.target.value)}
          placeholder="203.0.113.0/24"
          aria-label="Network in CIDR form"
          className="font-mono sm:max-w-xs"
        />
        <Select value={action} onValueChange={(value) => setAction(value as IpRuleAction)}>
          <SelectTrigger className="w-full sm:w-32" aria-label="Rule action">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="block">Block</SelectItem>
            <SelectItem value="allow">Allow</SelectItem>
          </SelectContent>
        </Select>
        <Button type="submit" size="sm" className="sm:self-center" loading={create.isPending}>
          <Plus className="size-4" />
          Add rule
        </Button>
      </form>

      <DataTable
        columns={columns}
        data={query.data?.rules ?? []}
        loading={query.isLoading}
        error={query.isError ? "Could not load the IP rules." : null}
        onRetry={() => query.refetch()}
        empty={
          <EmptyState
            icon={Ban}
            title="No IP rules"
            description="Every client may connect. Add a rule to block abusive networks or pin trusted ones."
            className="rounded-none border-0 bg-transparent"
          />
        }
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title="Remove IP rule"
        description={
          pendingDelete
            ? `${pendingDelete.cidr} will no longer be ${pendingDelete.action === "allow" ? "allowed" : "blocked"}.`
            : undefined
        }
        confirmLabel="Remove"
        destructive
        closeOnConfirm={false}
        loading={remove.isPending}
        onConfirm={() => {
          if (pendingDelete) remove.mutate(pendingDelete.id);
        }}
      />
    </div>
  );
}
