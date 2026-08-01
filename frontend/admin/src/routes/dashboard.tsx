import { useQuery } from "@tanstack/react-query";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ErrorState,
  Skeleton,
  StatusDot,
  useAuth,
} from "@irixmail/shared";
import {
  Activity,
  ArrowUpCircle,
  Globe,
  HardDrive,
  Inbox,
  Package,
  RefreshCw,
  Send,
  ShieldCheck,
  Users,
  type LucideIcon,
} from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatBytes, formatNumber } from "@/lib/format";

interface DashboardData {
  version: { current: string; latest: string | null; updateAvailable: boolean };
  domains: number;
  accounts: number;
  messagesInToday: number;
  messagesOutToday: number;
  queueDepth: number;
  storageBytes: number;
  recentLogEntries: number;
  certificate: { status: string; expiresAt: number | null };
  dns: { status: string };
  services: Array<{ name: string; status: string }>;
}

type Tone = "neutral" | "success" | "warning" | "danger" | "info";

function statusTone(status: string): Tone {
  switch (status) {
    case "ok":
    case "valid":
    case "running":
    case "healthy":
      return "success";
    case "warning":
    case "degraded":
    case "expiring":
      return "warning";
    case "error":
    case "failed":
    case "expired":
    case "stopped":
      return "danger";
    default:
      return "neutral";
  }
}

function StatCard({ icon: Icon, label, value }: { icon: LucideIcon; label: string; value: string }) {
  return (
    <Card className="py-0">
      <CardContent className="flex items-start justify-between gap-4 p-5">
        <div className="space-y-1">
          <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">{label}</p>
          <p className="text-2xl font-semibold tabular-nums">{value}</p>
        </div>
        <div className="flex size-9 items-center justify-center rounded-md border bg-muted/40 text-primary">
          <Icon className="size-5" />
        </div>
      </CardContent>
    </Card>
  );
}

export function DashboardPage() {
  const { client } = useAuth();
  const query = useQuery({
    queryKey: ["dashboard"],
    queryFn: ({ signal }) => client.get<DashboardData>("/api/dashboard", { signal }),
  });

  return (
    <div>
      <PageHeader
        title="Dashboard"
        description="Server health at a glance"
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={() => query.refetch()}
            loading={query.isFetching}
          >
            <RefreshCw className="size-4" />
            Refresh
          </Button>
        }
      />

      {query.isError ? (
        <ErrorState
          description="Could not load the dashboard."
          onRetry={() => query.refetch()}
        />
      ) : query.isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 6 }).map((_, index) => (
            <Skeleton key={index} className="h-24" />
          ))}
        </div>
      ) : query.data ? (
        <div className="space-y-4">
          {query.data.version?.updateAvailable ? (
            <Card className="border-primary/40 py-0">
              <CardContent className="flex items-start gap-4 p-5">
                <div className="flex size-9 shrink-0 items-center justify-center rounded-md border bg-muted/40 text-primary">
                  <ArrowUpCircle className="size-5" />
                </div>
                <div className="space-y-1">
                  <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">
                    Update available
                  </p>
                  <p className="text-sm">
                    irixmail {query.data.version.latest} is out; this server runs v
                    {query.data.version.current}.
                  </p>
                  <p className="text-sm text-muted-foreground">
                    Run{" "}
                    <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs">
                      sudo irixmail update
                    </code>{" "}
                    on the server, or let the daily auto-update install it.
                  </p>
                </div>
              </CardContent>
            </Card>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <StatCard icon={Globe} label="Domains" value={formatNumber(query.data.domains)} />
            <StatCard icon={Users} label="Accounts" value={formatNumber(query.data.accounts)} />
            <StatCard icon={Send} label="Queue depth" value={formatNumber(query.data.queueDepth)} />
            <StatCard icon={Inbox} label="Received today" value={formatNumber(query.data.messagesInToday)} />
            <StatCard icon={Activity} label="Sent today" value={formatNumber(query.data.messagesOutToday)} />
            <StatCard icon={HardDrive} label="Storage used" value={formatBytes(query.data.storageBytes)} />
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <Card className="py-0">
              <CardContent className="space-y-3 p-5">
                <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">Posture</p>
                <div className="flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm">
                    <ShieldCheck className="size-4 text-muted-foreground" />
                    Certificate
                  </span>
                  <Badge variant={statusTone(query.data.certificate.status) === "success" ? "success" : "muted"}>
                    {query.data.certificate.status}
                  </Badge>
                </div>
                <div className="flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm">
                    <Globe className="size-4 text-muted-foreground" />
                    DNS health
                  </span>
                  <Badge variant={statusTone(query.data.dns.status) === "success" ? "success" : "muted"}>
                    {query.data.dns.status}
                  </Badge>
                </div>
                <div className="flex items-center justify-between">
                  <span className="flex items-center gap-2 text-sm">
                    <Package className="size-4 text-muted-foreground" />
                    Server version
                  </span>
                  <Badge variant="muted">v{query.data.version?.current ?? "?"}</Badge>
                </div>
              </CardContent>
            </Card>

            <Card className="py-0">
              <CardContent className="space-y-3 p-5">
                <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">Services</p>
                {query.data.services.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No services reporting.</p>
                ) : (
                  <ul className="space-y-2">
                    {query.data.services.map((service) => (
                      <li key={service.name} className="flex items-center justify-between text-sm">
                        <span>{service.name}</span>
                        <span className="flex items-center gap-2 text-muted-foreground">
                          <StatusDot tone={statusTone(service.status)} />
                          {service.status}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </div>
        </div>
      ) : null}
    </div>
  );
}
