import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  Button,
  Card,
  CardContent,
  ConfirmDialog,
  EmptyState,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormMessage,
  Input,
  toast,
  useAuth,
} from "@irixmail/shared";
import { AtSign, Plus, Trash2 } from "lucide-react";

import type { Domain } from "@/lib/types";

const schema = z.object({
  alias: z
    .string()
    .min(1, "Alias domain is required")
    .regex(/^(?!-)[a-z0-9-]+(\.[a-z0-9-]+)+$/i, "Enter a valid domain"),
});
type FormValues = z.infer<typeof schema>;

export function DomainAliasesTab({ domain }: { domain: Domain }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [pendingRemoval, setPendingRemoval] = React.useState<string | null>(null);

  const save = useMutation({
    mutationFn: (aliases: string[]) =>
      client.put<{ domain: Domain }>(`/api/domains/${domain.id}`, { aliases }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["domain", domain.id] });
      void queryClient.invalidateQueries({ queryKey: ["domains"] });
    },
    onError: () => toast.error("Could not update aliases"),
  });

  const form = useForm<FormValues>({ resolver: zodResolver(schema), defaultValues: { alias: "" } });

  const onAdd = form.handleSubmit((values) => {
    const alias = values.alias.trim().toLowerCase();
    if (domain.aliases.includes(alias) || alias === domain.name) {
      form.setError("alias", { message: "This domain is already listed" });
      return;
    }
    save.mutate([...domain.aliases, alias], { onSuccess: () => form.reset() });
  });

  const onRemove = (alias: string) => {
    save.mutate(
      domain.aliases.filter((entry) => entry !== alias),
      {
        onSuccess: () => {
          setPendingRemoval(null);
          toast.success("Alias removed");
        },
      },
    );
  };

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
                  <Input placeholder="alias.example.com" className="font-mono" {...field} />
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

      {domain.aliases.length === 0 ? (
        <EmptyState icon={AtSign} title="No alias domains" description="Add an alternative domain name that delivers here." />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {domain.aliases.map((alias) => (
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
        title="Remove alias domain"
        description={pendingRemoval ? `Mail will no longer be accepted for ${pendingRemoval}.` : undefined}
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
