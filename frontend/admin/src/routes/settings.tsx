import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  ErrorState,
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Skeleton,
  Textarea,
  toast,
  useAuth,
} from "@irixmail/shared";

import { PageHeader } from "@/components/page-header";

export interface SettingsData {
  hostname: string;
  antiSpam: { dnsblZones: string[]; greylistWindowSeconds: number };
  rateLimits: {
    maxConnectionsPerIp: number;
    maxMessagesPerConnection: number;
    maxMessagesPerSenderPerHour: number;
    maxMessagesPerDomainPerHour: number;
  };
  listeners: {
    smtp: number;
    submission: number[];
    imap: number[];
    pop3: number[];
    https: number;
    http: number;
  };
}

export type SettingsPatch = Partial<Omit<SettingsData, "hostname" | "listeners">>;

export function useSettingsSave() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: SettingsPatch) => client.put("/api/settings", body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["settings"] });
      toast.success("Settings saved");
    },
    onError: () => toast.error("Could not save settings"),
  });
}

type ServerValues = { hostname: string };

function ServerSection({ settings }: { settings: SettingsData }) {
  const form = useForm<ServerValues>({
    values: { hostname: settings.hostname },
  });

  const ports = [
    { label: "SMTP", values: [settings.listeners.smtp] },
    { label: "Submission", values: settings.listeners.submission },
    { label: "IMAP", values: settings.listeners.imap },
    { label: "POP3", values: settings.listeners.pop3 },
    { label: "HTTPS", values: [settings.listeners.https] },
    { label: "HTTP", values: [settings.listeners.http] },
  ];

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Server</CardTitle>
        </CardHeader>
        <CardContent>
          <Form {...form}>
            <div className="grid gap-4">
              <FormField
                control={form.control}
                name="hostname"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Hostname</FormLabel>
                    <FormControl>
                      <Input className="font-mono" {...field} disabled readOnly />
                    </FormControl>
                    <FormDescription>
                      Set during server setup and cannot be changed here.
                    </FormDescription>
                  </FormItem>
                )}
              />
            </div>
          </Form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Listeners</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-2">
          {ports.map((entry) => (
            <div key={entry.label} className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">{entry.label}</span>
              <span className="flex gap-1">
                {entry.values.map((port) => (
                  <Badge key={port} variant="outline" className="font-mono">
                    {port}
                  </Badge>
                ))}
              </span>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

const antiSpamSchema = z.object({
  dnsblZones: z.string(),
  greylistWindowSeconds: z.string().regex(/^\d+$/, "Enter a whole number of seconds"),
});
type AntiSpamValues = z.infer<typeof antiSpamSchema>;

function AntiSpamSection({ settings }: { settings: SettingsData }) {
  const save = useSettingsSave();
  const form = useForm<AntiSpamValues>({
    resolver: zodResolver(antiSpamSchema),
    defaultValues: {
      dnsblZones: settings.antiSpam.dnsblZones.join("\n"),
      greylistWindowSeconds: String(settings.antiSpam.greylistWindowSeconds),
    },
  });

  const onSubmit = form.handleSubmit((values) => {
    const dnsblZones = values.dnsblZones
      .split(/[\s,]+/)
      .map((zone) => zone.trim().toLowerCase())
      .filter(Boolean);
    save.mutate({
      antiSpam: {
        dnsblZones,
        greylistWindowSeconds: Number(values.greylistWindowSeconds),
      },
    });
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>Anti-spam</CardTitle>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={onSubmit} className="grid gap-4" noValidate>
            <FormField
              control={form.control}
              name="dnsblZones"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>DNSBL zones</FormLabel>
                  <FormControl>
                    <Textarea rows={3} className="font-mono text-xs" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="greylistWindowSeconds"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Greylist memory (seconds)</FormLabel>
                  <FormControl>
                    <Input type="number" min={0} {...field} />
                  </FormControl>
                  <FormDescription>
                    How long a challenged sender and recipient pair is remembered. 0 disables
                    greylisting.
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="flex justify-end">
              <Button type="submit" loading={save.isPending}>
                Save
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}

const rateLimitsSchema = z.object({
  maxConnectionsPerIp: z.string().regex(/^\d+$/, "Enter a whole number"),
  maxMessagesPerConnection: z.string().regex(/^\d+$/, "Enter a whole number"),
  maxMessagesPerSenderPerHour: z.string().regex(/^\d+$/, "Enter a whole number"),
  maxMessagesPerDomainPerHour: z.string().regex(/^\d+$/, "Enter a whole number"),
});
type RateLimitsValues = z.infer<typeof rateLimitsSchema>;

function RateLimitsSection({ settings }: { settings: SettingsData }) {
  const save = useSettingsSave();
  const form = useForm<RateLimitsValues>({
    resolver: zodResolver(rateLimitsSchema),
    defaultValues: {
      maxConnectionsPerIp: String(settings.rateLimits.maxConnectionsPerIp),
      maxMessagesPerConnection: String(settings.rateLimits.maxMessagesPerConnection),
      maxMessagesPerSenderPerHour: String(settings.rateLimits.maxMessagesPerSenderPerHour),
      maxMessagesPerDomainPerHour: String(settings.rateLimits.maxMessagesPerDomainPerHour),
    },
  });

  const onSubmit = form.handleSubmit((values) =>
    save.mutate({
      rateLimits: {
        maxConnectionsPerIp: Number(values.maxConnectionsPerIp),
        maxMessagesPerConnection: Number(values.maxMessagesPerConnection),
        maxMessagesPerSenderPerHour: Number(values.maxMessagesPerSenderPerHour),
        maxMessagesPerDomainPerHour: Number(values.maxMessagesPerDomainPerHour),
      },
    }),
  );

  return (
    <Card>
      <CardHeader>
        <CardTitle>Rate limits</CardTitle>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={onSubmit} className="grid gap-4" noValidate>
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="maxConnectionsPerIp"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Max connections per IP</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="maxMessagesPerConnection"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Max messages per connection</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="maxMessagesPerSenderPerHour"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Max messages per sender per hour</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="maxMessagesPerDomainPerHour"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Max messages per domain per hour</FormLabel>
                    <FormControl>
                      <Input type="number" min={0} {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>
            <div className="flex justify-end">
              <Button type="submit" loading={save.isPending}>
                Save
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}

export function SettingsPage() {
  const { client } = useAuth();
  const query = useQuery({
    queryKey: ["settings"],
    queryFn: ({ signal }) => client.get<SettingsData>("/api/settings", { signal }),
  });

  return (
    <div className="mx-auto max-w-2xl">
      <PageHeader title="Settings" description="Server-wide configuration" />
      {query.isError ? (
        <ErrorState description="Could not load settings." onRetry={() => query.refetch()} />
      ) : query.isLoading || !query.data ? (
        <Skeleton className="h-64 w-full" />
      ) : (
        <div className="space-y-6">
          <ServerSection settings={query.data} />
          <AntiSpamSection settings={query.data} />
          <RateLimitsSection settings={query.data} />
        </div>
      )}
    </div>
  );
}
