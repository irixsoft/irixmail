import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Badge,
  Button,
  Card,
  CardContent,
  EmptyState,
  ErrorState,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  useAuth,
} from "@irixmail/shared";
import { RefreshCw, ScrollText, Search } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatDateTime } from "@/lib/format";

interface LogEntry {
  timestamp: number;
  severity: string;
  source: string;
  message: string;
}

const RANGE_HOURS: Record<string, number | null> = {
  "1h": 1,
  "6h": 6,
  "24h": 24,
  "7d": 168,
  all: null,
};

function severityVariant(severity: string): "destructive" | "warning" | "info" | "muted" {
  switch (severity.toLowerCase()) {
    case "error":
      return "destructive";
    case "warn":
    case "warning":
      return "warning";
    case "info":
      return "info";
    default:
      return "muted";
  }
}

export function LogsPage() {
  const { client } = useAuth();
  const [severity, setSeverity] = React.useState("all");
  const [range, setRange] = React.useState("24h");
  const [searchInput, setSearchInput] = React.useState("");
  const [search, setSearch] = React.useState("");

  const since = React.useMemo(() => {
    const hours = RANGE_HOURS[range];
    if (!hours) return undefined;
    return Date.now() - hours * 60 * 60 * 1000;
  }, [range]);

  const query = useQuery({
    queryKey: ["logs", severity, search, range],
    queryFn: ({ signal }) =>
      client.get<{ logs: LogEntry[] }>("/api/logs", {
        signal,
        query: {
          severity: severity === "all" ? undefined : severity,
          search: search || undefined,
          since,
        },
      }),
  });

  const logs = query.data?.logs ?? [];

  return (
    <div>
      <PageHeader title="Logs" description="Recent server events" />

      <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center">
        <Select value={severity} onValueChange={setSeverity}>
          <SelectTrigger className="w-full sm:w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All severities</SelectItem>
            <SelectItem value="info">Info</SelectItem>
            <SelectItem value="warn">Warning</SelectItem>
            <SelectItem value="error">Error</SelectItem>
          </SelectContent>
        </Select>
        <Select value={range} onValueChange={setRange}>
          <SelectTrigger className="w-full sm:w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="1h">Last hour</SelectItem>
            <SelectItem value="6h">Last 6 hours</SelectItem>
            <SelectItem value="24h">Last 24 hours</SelectItem>
            <SelectItem value="7d">Last 7 days</SelectItem>
            <SelectItem value="all">All time</SelectItem>
          </SelectContent>
        </Select>
        <form
          className="relative flex-1"
          onSubmit={(event) => {
            event.preventDefault();
            setSearch(searchInput.trim());
          }}
        >
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={searchInput}
            onChange={(event) => setSearchInput(event.target.value)}
            placeholder="Search messages"
            className="pl-9"
          />
        </form>
        <Button variant="outline" size="icon" aria-label="Refresh" onClick={() => query.refetch()}>
          <RefreshCw className="size-4" />
        </Button>
      </div>

      {query.isError ? (
        <ErrorState description="Could not load logs." onRetry={() => query.refetch()} />
      ) : query.isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 8 }).map((_, index) => (
            <Skeleton key={index} className="h-10 w-full" />
          ))}
        </div>
      ) : logs.length === 0 ? (
        <EmptyState icon={ScrollText} title="No log entries" description="Nothing matches these filters." />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {logs.map((entry, index) => (
              <div key={`${entry.timestamp}-${index}`} className="flex items-start gap-3 p-3 text-sm">
                <Badge variant={severityVariant(entry.severity)} className="mt-0.5 shrink-0 uppercase">
                  {entry.severity}
                </Badge>
                <div className="min-w-0 flex-1">
                  <p className="font-mono break-words">{entry.message}</p>
                  <p className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                    {formatDateTime(entry.timestamp)} · {entry.source}
                  </p>
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
