import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  toast,
  useAuth,
} from "@irixmail/shared";
import { ArrowLeft } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import type { Account, Domain } from "@/lib/types";

const schema = z.object({
  domainId: z.string().min(1, "Choose a domain"),
  localPart: z
    .string()
    .min(1, "Username is required")
    .regex(/^[a-z0-9._%+-]+$/i, "Use letters, numbers and . _ % + -"),
  displayName: z.string().optional(),
  role: z.enum(["user", "admin"]),
  password: z.string().optional(),
});
type FormValues = z.infer<typeof schema>;

export function AccountCreatePage() {
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const domainsQuery = useQuery({
    queryKey: ["domains"],
    queryFn: ({ signal }) => client.get<{ domains: Domain[] }>("/api/domains", { signal }),
  });

  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: { domainId: "", localPart: "", displayName: "", role: "user", password: "" },
  });

  const create = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      client.post<{ account: Account }>("/api/accounts", body),
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: ["accounts"] });
      toast.success("Account created");
      navigate(`/accounts/${data.account.id}`);
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        form.setError("localPart", { message: "This address already exists" });
      } else {
        toast.error("Could not create the account");
      }
    },
  });

  const onSubmit = form.handleSubmit((values) => {
    const password = values.password?.trim();
    create.mutate({
      localPart: values.localPart.trim().toLowerCase(),
      domainId: values.domainId,
      displayName: values.displayName?.trim() ?? "",
      role: values.role,
      ...(password ? { password } : {}),
    });
  });

  const domains = domainsQuery.data?.domains ?? [];

  return (
    <div className="mx-auto max-w-xl">
      <Button variant="ghost" size="sm" className="mb-2" onClick={() => navigate("/accounts")}>
        <ArrowLeft className="size-4" />
        Accounts
      </Button>
      <PageHeader title="Add account" description="Create a new mailbox owner." />
      <Card>
        <CardContent>
          <Form {...form}>
            <form onSubmit={onSubmit} className="grid gap-4" noValidate>
              <div className="grid gap-4 sm:grid-cols-[1fr_auto_1fr] sm:items-start">
                <FormField
                  control={form.control}
                  name="localPart"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Username</FormLabel>
                      <FormControl>
                        <Input placeholder="alice" autoFocus className="font-mono" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <div className="hidden pt-8 text-muted-foreground sm:block">@</div>
                <FormField
                  control={form.control}
                  name="domainId"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Domain</FormLabel>
                      <Select value={field.value} onValueChange={field.onChange}>
                        <FormControl>
                          <SelectTrigger className="w-full">
                            <SelectValue placeholder={domains.length ? "Select" : "No domains"} />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {domains.map((domain) => (
                            <SelectItem key={domain.id} value={domain.id}>
                              {domain.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <FormField
                control={form.control}
                name="displayName"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Display name</FormLabel>
                    <FormControl>
                      <Input placeholder="Alice Example" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="role"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Role</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="user">User</SelectItem>
                        <SelectItem value="admin">Administrator</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Password</FormLabel>
                    <FormControl>
                      <Input type="password" autoComplete="new-password" {...field} />
                    </FormControl>
                    <FormDescription>Optional — you can set this later.</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={() => navigate("/accounts")}>
                  Cancel
                </Button>
                <Button type="submit" loading={create.isPending} disabled={domains.length === 0}>
                  Create account
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  );
}
