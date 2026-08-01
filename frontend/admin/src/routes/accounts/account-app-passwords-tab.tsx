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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
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
import { KeyRound, Plus, Trash2 } from "lucide-react";

import { CopyButton } from "@/components/copy-button";
import { formatDate, formatDateTime } from "@/lib/format";
import type { AppPassword } from "@/lib/types";

const schema = z.object({ name: z.string().min(1, "Give it a name") });
type FormValues = z.infer<typeof schema>;

interface Revealed {
  name: string;
  password: string;
}

export function AccountAppPasswordsTab({ accountId }: { accountId: string }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [revealed, setRevealed] = React.useState<Revealed | null>(null);
  const [pendingRevoke, setPendingRevoke] = React.useState<AppPassword | null>(null);

  const query = useQuery({
    queryKey: ["account", accountId, "app-passwords"],
    queryFn: ({ signal }) =>
      client.get<{ appPasswords: AppPassword[] }>(`/api/accounts/${accountId}/app-passwords`, {
        signal,
      }),
  });

  const create = useMutation({
    mutationFn: (name: string) =>
      client.post<{ id: string; name: string; password: string }>(
        `/api/accounts/${accountId}/app-passwords`,
        { name },
      ),
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: ["account", accountId, "app-passwords"] });
      setRevealed({ name: data.name, password: data.password });
    },
    onError: () => toast.error("Could not create the app password"),
  });

  const revoke = useMutation({
    mutationFn: (id: string) => client.delete(`/api/accounts/${accountId}/app-passwords/${id}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["account", accountId, "app-passwords"] });
      setPendingRevoke(null);
      toast.success("App password revoked");
    },
    onError: () => toast.error("Could not revoke the app password"),
  });

  const form = useForm<FormValues>({ resolver: zodResolver(schema), defaultValues: { name: "" } });

  const onCreate = form.handleSubmit((values) => {
    create.mutate(values.name.trim(), { onSuccess: () => form.reset() });
  });

  if (query.isError) {
    return <ErrorState description="Could not load app passwords." onRetry={() => query.refetch()} />;
  }

  const items = query.data?.appPasswords ?? [];

  return (
    <div className="space-y-4">
      <Form {...form}>
        <form onSubmit={onCreate} className="flex items-start gap-2">
          <FormField
            control={form.control}
            name="name"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormControl>
                  <Input placeholder="e.g. iPhone Mail" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <Button type="submit" loading={create.isPending}>
            <Plus className="size-4" />
            Create
          </Button>
        </form>
      </Form>

      {query.isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : items.length === 0 ? (
        <EmptyState
          icon={KeyRound}
          title="No app passwords"
          description="App passwords let mail clients sign in without your main password."
        />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {items.map((item) => (
              <div key={item.id} className="flex items-center justify-between gap-3 p-3">
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{item.name}</p>
                  <p className="text-xs text-muted-foreground">
                    Created {formatDate(item.createdAt)} ·{" "}
                    {item.lastUsedAt ? `last used ${formatDateTime(item.lastUsedAt)}` : "never used"}
                  </p>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Revoke ${item.name}`}
                  onClick={() => setPendingRevoke(item)}
                >
                  <Trash2 className="size-4 text-destructive" />
                </Button>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      <Dialog
        open={revealed !== null}
        onOpenChange={(open) => {
          if (!open) setRevealed(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>App password created</DialogTitle>
            <DialogDescription>
              Copy it now — for security, you won&apos;t be able to see it again.
            </DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2 rounded-md border bg-muted/40 p-3">
            <code className="flex-1 font-mono text-sm break-all">{revealed?.password}</code>
            <CopyButton value={revealed?.password ?? ""} label="Copy app password" />
          </div>
          <DialogFooter>
            <Button onClick={() => setRevealed(null)}>Done</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={pendingRevoke !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRevoke(null);
        }}
        title="Revoke app password"
        description={
          pendingRevoke ? `"${pendingRevoke.name}" will stop working immediately.` : undefined
        }
        confirmLabel="Revoke"
        destructive
        closeOnConfirm={false}
        loading={revoke.isPending}
        onConfirm={() => {
          if (pendingRevoke) revoke.mutate(pendingRevoke.id);
        }}
      />
    </div>
  );
}
