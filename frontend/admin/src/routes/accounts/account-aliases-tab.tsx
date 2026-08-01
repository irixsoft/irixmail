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
  toast,
  useAuth,
} from "@irixmail/shared";
import { AtSign, Plus, Trash2 } from "lucide-react";

const schema = z.object({ alias: z.email("Enter a valid email address") });
type FormValues = z.infer<typeof schema>;

export function AccountAliasesTab({ accountId }: { accountId: string }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [pendingRemoval, setPendingRemoval] = React.useState<string | null>(null);

  const query = useQuery({
    queryKey: ["account", accountId, "aliases"],
    queryFn: ({ signal }) =>
      client.get<{ aliases: string[] }>(`/api/accounts/${accountId}/aliases`, { signal }),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["account", accountId, "aliases"] });
    void queryClient.invalidateQueries({ queryKey: ["account", accountId] });
  };

  const add = useMutation({
    mutationFn: (alias: string) =>
      client.post<{ aliases: string[] }>(`/api/accounts/${accountId}/aliases`, { alias }),
    onSuccess: invalidate,
    onError: () => toast.error("Could not add the alias"),
  });

  const remove = useMutation({
    mutationFn: (alias: string) =>
      client.delete(`/api/accounts/${accountId}/aliases/${encodeURIComponent(alias)}`),
    onSuccess: () => {
      invalidate();
      setPendingRemoval(null);
      toast.success("Alias removed");
    },
    onError: () => toast.error("Could not remove the alias"),
  });

  const form = useForm<FormValues>({ resolver: zodResolver(schema), defaultValues: { alias: "" } });

  const onAdd = form.handleSubmit((values) => {
    const alias = values.alias.trim().toLowerCase();
    if ((query.data?.aliases ?? []).includes(alias)) {
      form.setError("alias", { message: "This alias already exists" });
      return;
    }
    add.mutate(alias, { onSuccess: () => form.reset() });
  });

  if (query.isError) {
    return <ErrorState description="Could not load aliases." onRetry={() => query.refetch()} />;
  }

  const aliases = query.data?.aliases ?? [];

  return (
    <div className="space-y-4">
      <Form {...form}>
        <form onSubmit={onAdd} className="flex items-start gap-2">
          <FormField
            control={form.control}
            name="alias"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormControl>
                  <Input placeholder="alias@example.com" className="font-mono" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <Button type="submit" loading={add.isPending}>
            <Plus className="size-4" />
            Add
          </Button>
        </form>
      </Form>

      {query.isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : aliases.length === 0 ? (
        <EmptyState
          icon={AtSign}
          title="No aliases"
          description="Add another address that delivers to this mailbox."
        />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {aliases.map((alias) => (
              <div key={alias} className="flex items-center justify-between gap-3 p-3">
                <span className="font-mono text-sm">{alias}</span>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove ${alias}`}
                  onClick={() => setPendingRemoval(alias)}
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
        title="Remove alias"
        description={pendingRemoval ? `${pendingRemoval} will no longer deliver here.` : undefined}
        confirmLabel="Remove"
        destructive
        closeOnConfirm={false}
        loading={remove.isPending}
        onConfirm={() => {
          if (pendingRemoval) remove.mutate(pendingRemoval);
        }}
      />
    </div>
  );
}
