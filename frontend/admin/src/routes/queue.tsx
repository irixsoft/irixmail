import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ConfirmDialog,
  EmptyState,
  ErrorState,
  Skeleton,
  StatusDot,
  toast,
  useAuth,
} from "@irixmail/shared";
import { Inbox, RefreshCw, RotateCw, Trash2 } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatDateTime } from "@/lib/format";
import type { QueueMessage } from "@/lib/types";

function recipientTone(status: string): "neutral" | "success" | "warning" | "danger" {
  switch (status) {
    case "delivered":
    case "sent":
      return "success";
    case "failed":
    case "bounced":
      return "danger";
    case "deferred":
    case "retrying":
      return "warning";
    default:
      return "neutral";
  }
}

export function QueuePage() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [pendingDelete, setPendingDelete] = React.useState<string | null>(null);
  const query = useQuery({
    queryKey: ["queue"],
    queryFn: ({ signal }) => client.get<{ queue: QueueMessage[] }>("/api/queue", { signal }),
  });

  const retry = useMutation({
    mutationFn: (id: string) => client.post(`/api/queue/${encodeURIComponent(id)}/retry`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["queue"] });
      toast.success("Retry scheduled");
    },
    onError: () => toast.error("Could not schedule a retry"),
  });

  const remove = useMutation({
    mutationFn: (id: string) => client.delete(`/api/queue/${encodeURIComponent(id)}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["queue"] });
      setPendingDelete(null);
      toast.success("Message removed from the queue");
    },
    onError: () => toast.error("Could not delete the message"),
  });

  const messages = query.data?.queue ?? [];

  return (
    <div>
      <PageHeader
        title="Outbound queue"
        description="Messages waiting for delivery"
        actions={
          <Button variant="outline" size="sm" onClick={() => query.refetch()} loading={query.isFetching}>
            <RefreshCw className="size-4" />
            Refresh
          </Button>
        }
      />

      {query.isError ? (
        <ErrorState description="Could not load the queue." onRetry={() => query.refetch()} />
      ) : query.isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-28 w-full" />
          ))}
        </div>
      ) : messages.length === 0 ? (
        <EmptyState
          icon={Inbox}
          title="The queue is empty"
          description="Outbound messages awaiting delivery will appear here."
        />
      ) : (
        <div className="space-y-3">
          {messages.map((message) => (
            <Card key={message.id} className="py-0">
              <CardContent className="space-y-3 p-5">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate font-mono text-sm">{message.sender ?? "—"}</p>
                    <p className="text-xs text-muted-foreground">
                      {message.subject ?? "(no subject)"}
                      {message.createdAt ? ` · queued ${formatDateTime(message.createdAt)}` : ""}
                    </p>
                  </div>
                  <Badge variant="muted">{message.status ?? "pending"}</Badge>
                </div>
                <ul className="space-y-2 border-t pt-3">
                  {(message.recipients ?? []).map((recipient) => (
                    <li key={recipient.address} className="flex flex-col gap-1 text-sm">
                      <div className="flex items-center justify-between gap-2">
                        <span className="truncate font-mono">{recipient.address}</span>
                        <span className="flex items-center gap-2 text-muted-foreground">
                          <StatusDot tone={recipientTone(recipient.status)} />
                          {recipient.status}
                        </span>
                      </div>
                      {recipient.lastError ? (
                        <p className="font-mono text-xs text-destructive">{recipient.lastError}</p>
                      ) : null}
                    </li>
                  ))}
                </ul>
                <div className="flex justify-end gap-2 border-t pt-3">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => retry.mutate(message.id)}
                    loading={retry.isPending && retry.variables === message.id}
                  >
                    <RotateCw className="size-4" />
                    Retry now
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setPendingDelete(message.id)}
                  >
                    <Trash2 className="size-4 text-destructive" />
                    Delete
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title="Delete queued message"
        description="The message will be removed from the queue and not delivered."
        confirmLabel="Delete"
        destructive
        closeOnConfirm={false}
        loading={remove.isPending}
        onConfirm={() => {
          if (pendingDelete) remove.mutate(pendingDelete);
        }}
      />
    </div>
  );
}
