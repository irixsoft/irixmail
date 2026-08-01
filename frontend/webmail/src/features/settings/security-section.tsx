import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  ApiError,
  Badge,
  Button,
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
  FormLabel,
  FormMessage,
  Input,
  Label,
  Skeleton,
  toast,
  useAuth,
} from "@irixmail/shared";
import { KeyRound, Plus, Trash2 } from "lucide-react";

import { CopyField } from "@/components/copy-field";
import { formatDateTime } from "@/lib/format";
import { SettingsCard } from "./section-card";

export function SecuritySection() {
  return (
    <div className="space-y-4">
      <PasswordCard />
      <AppPasswordsCard />
      <TotpCard />
    </div>
  );
}

const passwordSchema = z
  .object({
    current: z.string().min(1, "Enter your current password"),
    next: z.string().min(8, "Use at least 8 characters"),
    confirm: z.string(),
  })
  .refine((values) => values.next === values.confirm, {
    message: "Passwords do not match",
    path: ["confirm"],
  });
type PasswordValues = z.infer<typeof passwordSchema>;

function PasswordCard() {
  const { client } = useAuth();
  const form = useForm<PasswordValues>({
    resolver: zodResolver(passwordSchema),
    defaultValues: { current: "", next: "", confirm: "" },
  });

  const change = useMutation({
    mutationFn: (values: PasswordValues) =>
      client.put("/api/me/password", { currentPassword: values.current, newPassword: values.next }),
    onSuccess: () => {
      toast.success("Password changed");
      form.reset();
    },
    onError: (error) => {
      if (error instanceof ApiError && (error.status === 400 || error.status === 401)) {
        form.setError("current", { message: "Your current password is incorrect" });
      } else {
        toast.error("Could not change your password");
      }
    },
  });

  const onSubmit = form.handleSubmit((values) => change.mutate(values));

  return (
    <SettingsCard title="Password" description="Used to sign in to webmail.">
      <Form {...form}>
        <form onSubmit={onSubmit} className="grid gap-4" noValidate>
          <FormField
            control={form.control}
            name="current"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Current password</FormLabel>
                <FormControl>
                  <Input type="password" autoComplete="current-password" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="next"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>New password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="new-password" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="confirm"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Confirm new password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="new-password" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
          <div className="flex justify-end">
            <Button type="submit" loading={change.isPending}>
              Change password
            </Button>
          </div>
        </form>
      </Form>
    </SettingsCard>
  );
}

interface AppPassword {
  id: string;
  name: string;
  createdAt: number;
  lastUsedAt: number | null;
}

function AppPasswordsCard() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [name, setName] = React.useState("");
  const [revealed, setRevealed] = React.useState<string | null>(null);
  const [pendingRevoke, setPendingRevoke] = React.useState<AppPassword | null>(null);

  const query = useQuery({
    queryKey: ["me", "app-passwords"],
    queryFn: ({ signal }) =>
      client.get<{ appPasswords: AppPassword[] }>("/api/me/app-passwords", { signal }),
  });

  const create = useMutation({
    mutationFn: (appName: string) =>
      client.post<{ id: string; name: string; password: string }>("/api/me/app-passwords", {
        name: appName,
      }),
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: ["me", "app-passwords"] });
      setRevealed(data.password);
      setName("");
    },
    onError: () => toast.error("Could not create the app password"),
  });

  const revoke = useMutation({
    mutationFn: (id: string) => client.delete(`/api/me/app-passwords/${id}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["me", "app-passwords"] });
      setPendingRevoke(null);
      toast.success("App password revoked");
    },
    onError: () => toast.error("Could not revoke the app password"),
  });

  if (query.isError) {
    return <ErrorState description="Could not load app passwords." onRetry={() => query.refetch()} />;
  }

  const items = query.data?.appPasswords ?? [];

  return (
    <SettingsCard
      title="App passwords"
      description="One per mail client — revoking a password signs that client out."
      bodyClassName="p-0"
    >
      <form
        className="flex items-start gap-2 border-b p-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim()) create.mutate(name.trim());
        }}
      >
        <Input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="e.g. iPhone Mail"
          aria-label="App password name"
        />
        <Button type="submit" loading={create.isPending} disabled={!name.trim()}>
          <Plus className="size-4" />
          Create
        </Button>
      </form>

      {query.isLoading ? (
        <div className="p-4">
          <Skeleton className="h-20 w-full" />
        </div>
      ) : items.length === 0 ? (
        <EmptyState
          icon={KeyRound}
          title="No app passwords"
          description="Use app passwords to sign in from mail clients."
          className="border-0 bg-transparent py-8"
        />
      ) : (
        <ul className="divide-y">
          {items.map((item) => (
            <li key={item.id} className="flex items-center justify-between gap-3 px-4 py-3">
              <div className="min-w-0">
                <p className="truncate text-[13px] font-medium">{item.name}</p>
                <p className="font-mono text-xs text-muted-foreground">
                  Created {formatDateTime(item.createdAt)} ·{" "}
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
            </li>
          ))}
        </ul>
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
          <CopyField value={revealed ?? ""} />
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
        description={pendingRevoke ? `"${pendingRevoke.name}" will stop working immediately.` : undefined}
        confirmLabel="Revoke"
        destructive
        closeOnConfirm={false}
        loading={revoke.isPending}
        onConfirm={() => {
          if (pendingRevoke) revoke.mutate(pendingRevoke.id);
        }}
      />
    </SettingsCard>
  );
}

interface TotpStatus {
  enabled: boolean;
}

interface TotpSetup {
  secret: string;
  otpauthUrl?: string;
  qr?: string;
  recoveryCodes: string[];
}

function TotpCard() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const [setup, setSetup] = React.useState<TotpSetup | null>(null);
  const [code, setCode] = React.useState("");

  const status = useQuery({
    queryKey: ["me", "totp"],
    queryFn: ({ signal }) => client.get<TotpStatus>("/api/me/totp", { signal }),
  });

  const begin = useMutation({
    mutationFn: () => client.post<TotpSetup>("/api/me/totp/setup"),
    onSuccess: (data) => setSetup(data),
    onError: () => toast.error("Could not start 2FA setup"),
  });

  const verify = useMutation({
    mutationFn: () => client.post("/api/me/totp/verify", { code }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["me", "totp"] });
      setSetup(null);
      setCode("");
      toast.success("Two-factor authentication enabled");
    },
    onError: () => toast.error("That code did not match"),
  });

  const disable = useMutation({
    mutationFn: () => client.post("/api/me/totp/disable"),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["me", "totp"] });
      toast.success("Two-factor authentication disabled");
    },
    onError: () => toast.error("Could not disable 2FA"),
  });

  if (status.isLoading) {
    return <Skeleton className="h-44 w-full rounded-lg" />;
  }

  const enabled = status.data?.enabled ?? false;

  return (
    <SettingsCard
      title="Two-factor authentication"
      description="A code from your authenticator app on top of your password."
      action={<Badge variant={enabled ? "success" : "muted"}>{enabled ? "Enabled" : "Disabled"}</Badge>}
    >
      {enabled ? (
        <Button variant="outline" onClick={() => disable.mutate()} loading={disable.isPending}>
          Disable 2FA
        </Button>
      ) : setup ? (
        <div className="space-y-4">
          <p className="text-[13px] text-muted-foreground">
            Scan this with your authenticator app, then enter the six-digit code.
          </p>
          {setup.qr ? (
            <img src={setup.qr} alt="TOTP QR code" className="size-44 rounded-md border bg-white p-2" />
          ) : null}
          <div className="space-y-1.5">
            <Label className="text-[13px]">Secret</Label>
            <CopyField value={setup.secret} />
          </div>
          {setup.recoveryCodes.length > 0 ? (
            <div className="space-y-1.5">
              <Label className="text-[13px]">Recovery codes</Label>
              <div className="grid grid-cols-2 gap-2 rounded-md border bg-muted/40 p-3 font-mono text-sm">
                {setup.recoveryCodes.map((recovery) => (
                  <span key={recovery}>{recovery}</span>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                Store these somewhere safe — each one signs you in once.
              </p>
            </div>
          ) : null}
          <div className="flex items-end gap-2">
            <div className="grid flex-1 gap-1.5">
              <Label htmlFor="totp-code" className="text-[13px]">
                Verification code
              </Label>
              <Input
                id="totp-code"
                inputMode="numeric"
                maxLength={6}
                value={code}
                onChange={(event) => setCode(event.target.value)}
                className="font-mono tracking-[0.3em]"
              />
            </div>
            <Button onClick={() => verify.mutate()} loading={verify.isPending} disabled={code.length !== 6}>
              Verify
            </Button>
          </div>
        </div>
      ) : (
        <Button onClick={() => begin.mutate()} loading={begin.isPending}>
          Set up 2FA
        </Button>
      )}
    </SettingsCard>
  );
}
