import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  Button,
  Card,
  CardContent,
  ConfirmDialog,
  EmptyState,
  ErrorState,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormMessage,
  Input,
  Skeleton,
  Switch,
  toast,
  useAuth,
} from "@irixmail/shared";
import { Forward, Plus, Trash2 } from "lucide-react";

import type { Forwarding } from "@/lib/types";

const schema = z.object({ destination: z.email("Enter a valid email address") });
type FormValues = z.infer<typeof schema>;

interface SavePayload {
  destinations: string[];
  keepLocalCopy: boolean;
}

export function AccountForwardingTab({ accountId }: { accountId: string }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [pendingRemoval, setPendingRemoval] = React.useState<string | null>(null);

  const query = useQuery({
    queryKey: ["account", accountId, "forwarding"],
    queryFn: ({ signal }) =>
      client.get<{ forwarding: Forwarding }>(`/api/accounts/${accountId}/forwarding`, { signal }),
  });

  const save = useMutation({
    mutationFn: (payload: SavePayload) =>
      client.put<{ forwarding: Forwarding }>(`/api/accounts/${accountId}/forwarding`, payload),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["account", accountId, "forwarding"] }),
    onError: () => toast.error("Could not update forwarding"),
  });

  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: { destination: "" },
  });

  if (query.isError) {
    return <ErrorState description="Could not load forwarding." onRetry={() => query.refetch()} />;
  }
  if (query.isLoading || !query.data) {
    return <Skeleton className="h-40 w-full" />;
  }

  const forwarding = query.data.forwarding;

  const onAdd = form.handleSubmit((values) => {
    const destination = values.destination.trim().toLowerCase();
    if (forwarding.destinations.includes(destination)) {
      form.setError("destination", { message: "Already forwarding here" });
      return;
    }
    save.mutate(
      { destinations: [...forwarding.destinations, destination], keepLocalCopy: forwarding.keep_local_copy },
      { onSuccess: () => form.reset() },
    );
  });

  const onRemove = (destination: string) => {
    save.mutate(
      {
        destinations: forwarding.destinations.filter((entry) => entry !== destination),
        keepLocalCopy: forwarding.keep_local_copy,
      },
      {
        onSuccess: () => {
          setPendingRemoval(null);
          toast.success("Forwarding removed");
        },
      },
    );
  };

  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm font-medium">Keep a local copy</p>
            <p className="text-xs text-muted-foreground">
              Also store forwarded mail in this mailbox.
            </p>
          </div>
          <Switch
            checked={forwarding.keep_local_copy}
            aria-label="Keep a local copy"
            onCheckedChange={(checked) =>
              save.mutate({ destinations: forwarding.destinations, keepLocalCopy: checked })
            }
          />
        </CardContent>
      </Card>

      <Form {...form}>
        <form onSubmit={onAdd} className="flex items-start gap-2">
          <FormField
            control={form.control}
            name="destination"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormControl>
                  <Input placeholder="forward-to@example.com" className="font-mono" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <Button type="submit" loading={save.isPending}>
            <Plus className="size-4" />
            Add
          </Button>
        </form>
      </Form>

      {forwarding.destinations.length === 0 ? (
        <EmptyState
          icon={Forward}
          title="No forwarding addresses"
          description="Forward incoming mail to one or more external addresses."
        />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {forwarding.destinations.map((destination) => (
              <div key={destination} className="flex items-center justify-between gap-3 p-3">
                <span className="font-mono text-sm">{destination}</span>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove ${destination}`}
                  onClick={() => setPendingRemoval(destination)}
                >
                  <Trash2 className="size-4 text-destructive" />
                </Button>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={pendingRemoval !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRemoval(null);
        }}
        title="Remove forwarding"
        description={pendingRemoval ? `Mail will no longer forward to ${pendingRemoval}.` : undefined}
        confirmLabel="Remove"
        destructive
        closeOnConfirm={false}
        loading={save.isPending}
        onConfirm={() => {
          if (pendingRemoval) onRemove(pendingRemoval);
        }}
      />
    </div>
  );
}
