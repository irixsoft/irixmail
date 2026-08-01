import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "react-router-dom";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ErrorState,
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
  Skeleton,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
  toast,
  useAuth,
} from "@irixmail/shared";
import { ArrowLeft } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { AccountAliasesTab } from "@/routes/accounts/account-aliases-tab";
import { AccountAppPasswordsTab } from "@/routes/accounts/account-app-passwords-tab";
import { AccountForwardingTab } from "@/routes/accounts/account-forwarding-tab";
import { AccountSecurityTab } from "@/routes/accounts/account-security-tab";
import type { Account, Domain } from "@/lib/types";

const BYTES_PER_MB = 1024 * 1024;

const profileSchema = z.object({
  displayName: z.string().optional(),
  role: z.enum(["user", "admin"]),
  quotaMb: z.string().regex(/^\d+$/, "Enter a whole number"),
  quotaMessages: z.string().regex(/^\d+$/, "Enter a whole number"),
  signature: z.string().optional(),
});
type ProfileValues = z.infer<typeof profileSchema>;

const passwordSchema = z.object({
  password: z.string().min(8, "Use at least 8 characters"),
});
type PasswordValues = z.infer<typeof passwordSchema>;

function ProfileTab({ account }: { account: Account }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();

  const form = useForm<ProfileValues>({
    resolver: zodResolver(profileSchema),
    defaultValues: {
      displayName: account.display_name,
      role: account.role,
      quotaMb: String(Math.round(account.quota_bytes / BYTES_PER_MB)),
      quotaMessages: String(account.quota_messages),
      signature: account.signature,
    },
  });

  const passwordForm = useForm<PasswordValues>({
    resolver: zodResolver(passwordSchema),
    defaultValues: { password: "" },
  });

  const update = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      client.put<{ account: Account }>(`/api/accounts/${account.id}`, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["account", account.id] });
      void queryClient.invalidateQueries({ queryKey: ["accounts"] });
      toast.success("Account updated");
    },
    onError: () => toast.error("Could not update the account"),
  });

  const setPassword = useMutation({
    mutationFn: (password: string) =>
      client.put(`/api/accounts/${account.id}/password`, { password }),
    onSuccess: () => {
      toast.success("Password updated");
      passwordForm.reset();
    },
    onError: () => toast.error("Could not set the password"),
  });

  const onSave = form.handleSubmit((values) => {
    update.mutate({
      displayName: values.displayName ?? "",
      role: values.role,
      quotaBytes: Number(values.quotaMb) * BYTES_PER_MB,
      quotaMessages: Number(values.quotaMessages),
      signature: values.signature ?? "",
    });
  });

  const onSetPassword = passwordForm.handleSubmit((values) => setPassword.mutate(values.password));

  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm font-medium">Enabled</p>
            <p className="text-xs text-muted-foreground">Allow this account to sign in and receive mail</p>
          </div>
          <Switch
            checked={account.enabled}
            aria-label="Account enabled"
            onCheckedChange={(checked) => update.mutate({ enabled: checked })}
          />
        </CardContent>
      </Card>

      <Card>
        <CardContent>
          <Form {...form}>
            <form onSubmit={onSave} className="grid gap-4" noValidate>
              <FormField
                control={form.control}
                name="displayName"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Display name</FormLabel>
                    <FormControl>
                      <Input {...field} />
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
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="quotaMb"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Storage quota (MB)</FormLabel>
                      <FormControl>
                        <Input type="number" min={0} {...field} />
                      </FormControl>
                      <FormDescription>0 means unlimited.</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="quotaMessages"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Message quota</FormLabel>
                      <FormControl>
                        <Input type="number" min={0} {...field} />
                      </FormControl>
                      <FormDescription>0 means unlimited.</FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <FormField
                control={form.control}
                name="signature"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Signature</FormLabel>
                    <FormControl>
                      <Textarea rows={3} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div className="flex justify-end">
                <Button type="submit" loading={update.isPending}>
                  Save changes
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card>
        <CardContent>
          <Form {...passwordForm}>
            <form onSubmit={onSetPassword} className="grid gap-4" noValidate>
              <FormField
                control={passwordForm.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Set a new password</FormLabel>
                    <FormControl>
                      <Input type="password" autoComplete="new-password" {...field} />
                    </FormControl>
                    <FormDescription>The account owner can change this later.</FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div className="flex justify-end">
                <Button type="submit" variant="outline" loading={setPassword.isPending}>
                  Update password
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  );
}

export function AccountDetailPage() {
  const { id } = useParams();
  const accountId = id ?? "";
  const { client } = useAuth();
  const navigate = useNavigate();

  const query = useQuery({
    queryKey: ["account", accountId],
    queryFn: ({ signal }) =>
      client.get<{ account: Account }>(`/api/accounts/${accountId}`, { signal }),
    enabled: accountId !== "",
  });
  const domainsQuery = useQuery({
    queryKey: ["domains"],
    queryFn: ({ signal }) => client.get<{ domains: Domain[] }>("/api/domains", { signal }),
  });

  if (query.isError) {
    return (
      <div className="mx-auto max-w-3xl">
        <ErrorState description="Could not load this account." onRetry={() => query.refetch()} />
      </div>
    );
  }
  if (query.isLoading || !query.data) {
    return (
      <div className="mx-auto max-w-3xl space-y-4">
        <Skeleton className="h-8 w-56" />
        <Skeleton className="h-9 w-full max-w-md" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  const account = query.data.account;
  const domainName =
    (domainsQuery.data?.domains ?? []).find((domain) => domain.id === account.domain_id)?.name ??
    "";
  const address = `${account.local_part}@${domainName}`;

  return (
    <div className="mx-auto max-w-3xl">
      <Button variant="ghost" size="sm" className="mb-2" onClick={() => navigate("/accounts")}>
        <ArrowLeft className="size-4" />
        Accounts
      </Button>
      <PageHeader
        title={<span className="font-mono">{address}</span>}
        description={account.display_name || undefined}
        actions={account.role === "admin" ? <Badge>Admin</Badge> : <Badge variant="muted">User</Badge>}
      />

      <Tabs defaultValue="profile">
        <TabsList className="flex-wrap">
          <TabsTrigger value="profile">Profile</TabsTrigger>
          <TabsTrigger value="aliases">Aliases</TabsTrigger>
          <TabsTrigger value="forwarding">Forwarding</TabsTrigger>
          <TabsTrigger value="app-passwords">App passwords</TabsTrigger>
          <TabsTrigger value="security">Security</TabsTrigger>
        </TabsList>

        <TabsContent value="profile" className="pt-4">
          <ProfileTab account={account} />
        </TabsContent>
        <TabsContent value="aliases" className="pt-4">
          <AccountAliasesTab accountId={account.id} />
        </TabsContent>
        <TabsContent value="forwarding" className="pt-4">
          <AccountForwardingTab accountId={account.id} />
        </TabsContent>
        <TabsContent value="app-passwords" className="pt-4">
          <AccountAppPasswordsTab accountId={account.id} />
        </TabsContent>
        <TabsContent value="security" className="pt-4">
          <AccountSecurityTab accountId={account.id} accountLabel={address} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
