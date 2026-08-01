import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  ApiError,
  Button,
  Card,
  CardContent,
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  toast,
  useAuth,
} from "@irixmail/shared";
import { ArrowLeft } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import type { Domain } from "@/lib/types";

const schema = z.object({
  name: z
    .string()
    .min(1, "Domain name is required")
    .regex(/^(?!-)[a-z0-9-]+(\.[a-z0-9-]+)+$/i, "Enter a valid domain like example.com"),
  aliases: z.string().optional(),
});
type FormValues = z.infer<typeof schema>;

export function DomainCreatePage() {
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: { name: "", aliases: "" },
  });

  const create = useMutation({
    mutationFn: (body: { name: string; aliases: string[] }) =>
      client.post<{ domain: Domain }>("/api/domains", body),
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: ["domains"] });
      toast.success("Domain created — DKIM keys generated");
      navigate(`/domains/${data.domain.id}`);
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        form.setError("name", { message: "This domain already exists" });
      } else {
        toast.error("Could not create the domain");
      }
    },
  });

  const onSubmit = form.handleSubmit((values) => {
    const aliases = (values.aliases ?? "")
      .split(/[\s,]+/)
      .map((entry) => entry.trim().toLowerCase())
      .filter(Boolean);
    create.mutate({ name: values.name.trim().toLowerCase(), aliases });
  });

  return (
    <div className="mx-auto max-w-xl">
      <Button variant="ghost" size="sm" className="mb-2" onClick={() => navigate("/domains")}>
        <ArrowLeft className="size-4" />
        Domains
      </Button>
      <PageHeader title="Add domain" description="DKIM keys are generated automatically." />
      <Card>
        <CardContent>
          <Form {...form}>
            <form onSubmit={onSubmit} className="grid gap-4" noValidate>
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Domain name</FormLabel>
                    <FormControl>
                      <Input placeholder="example.com" autoFocus className="font-mono" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="aliases"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Alias domains</FormLabel>
                    <FormControl>
                      <Input placeholder="mail.example.com, example.net" className="font-mono" {...field} />
                    </FormControl>
                    <FormDescription>Optional. Separate multiple domains with commas.</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={() => navigate("/domains")}>
                  Cancel
                </Button>
                <Button type="submit" loading={create.isPending}>
                  Create domain
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  );
}
