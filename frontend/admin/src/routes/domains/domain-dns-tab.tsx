import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Badge,
  Button,
  Card,
  CardContent,
  ErrorState,
  Skeleton,
  StatusDot,
  toast,
  useAuth,
} from "@irixmail/shared";
import { Download, Info, RefreshCw } from "lucide-react";

import { CopyButton } from "@/components/copy-button";
import type { DnsRecord, DnsStatus, DnsVerifyResult } from "@/lib/types";

interface DnsResponse {
  domain: string;
  status: DnsStatus;
  records: DnsRecord[];
  zone: string;
}

interface VerifyResponse {
  domain: string;
  results: DnsVerifyResult[];
  allGreen: boolean;
}

function recordKey(record: DnsRecord): string {
  return `${record.kind}:${record.name}:${record.record_type}`;
}

function downloadZone(domain: string, zone: string) {
  const url = URL.createObjectURL(new Blob([zone], { type: "text/plain;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `${domain}.txt`;
  link.click();
  URL.revokeObjectURL(url);
}

function tone(status: string | undefined): "neutral" | "success" | "warning" {
  if (!status) return "neutral";
  return ["ok", "verified", "pass", "green"].includes(status) ? "success" : "warning";
}

function outOfZoneHost(records: DnsRecord[]): string | undefined {
  return records.find((record) => !record.in_zone)?.name;
}

export function DomainDnsTab({ domainId }: { domainId: string }) {
  const { client } = useAuth();
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["domain", domainId, "dns"],
    queryFn: ({ signal }) =>
      client.get<DnsResponse>(`/api/domains/${domainId}/dns`, { signal }),
  });

  const verify = useMutation({
    mutationFn: () => client.post<VerifyResponse>(`/api/domains/${domainId}/dns/verify`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["domains"] });
      void queryClient.invalidateQueries({ queryKey: ["domain", domainId] });
    },
    onError: () => toast.error("Could not verify DNS"),
  });

  const statusByRecord = new Map<string, string>();
  for (const result of verify.data?.results ?? []) {
    statusByRecord.set(recordKey(result.record), result.status);
  }

  const foreignHost = outOfZoneHost(query.data?.records ?? []);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <p className="text-sm text-muted-foreground">Set these records at your DNS provider.</p>
          {verify.data ? (
            <Badge variant={verify.data.allGreen ? "success" : "warning"}>
              {verify.data.allGreen ? "All verified" : "Needs attention"}
            </Badge>
          ) : null}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={query.isLoading || !query.data?.zone}
            onClick={() => {
              const data = query.data;
              if (data?.zone) downloadZone(data.domain, data.zone);
            }}
          >
            <Download className="size-4" />
            Download zone file
          </Button>
          <Button size="sm" variant="outline" onClick={() => verify.mutate()} loading={verify.isPending}>
            <RefreshCw className="size-4" />
            Check DNS
          </Button>
        </div>
      </div>

      <div className="flex items-start gap-2 rounded-md border border-info/30 bg-info/5 p-3 text-sm">
        <Info className="mt-0.5 size-4 shrink-0 text-info" />
        <p className="text-muted-foreground">
          Reverse DNS (PTR) is configured at your VPS provider, not here — but it strongly affects
          deliverability. Make sure your mail host&apos;s IP has a matching PTR record.
        </p>
      </div>

      {foreignHost ? (
        <div className="flex items-start gap-2 rounded-md border border-info/30 bg-info/5 p-3 text-sm">
          <Info className="mt-0.5 size-4 shrink-0 text-info" />
          <p className="text-muted-foreground">
            Records marked Other zone point at <span className="font-mono">{foreignHost}</span>, which
            this domain does not host. Publish them in the zone that does, not here — the zone file
            download leaves them out.
          </p>
        </div>
      ) : null}

      {query.isError ? (
        <ErrorState description="Could not load DNS records." onRetry={() => query.refetch()} />
      ) : query.isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, index) => (
            <Skeleton key={index} className="h-16 w-full" />
          ))}
        </div>
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {(query.data?.records ?? []).map((record) => {
              const status = statusByRecord.get(recordKey(record));
              return (
                <div key={recordKey(record)} className="flex items-start gap-3 p-4">
                  <StatusDot tone={record.in_zone ? tone(status) : "info"} className="mt-1.5" />
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex items-center gap-2">
                      <Badge variant="outline" className="font-mono">
                        {record.record_type}
                      </Badge>
                      <span className="truncate font-mono text-xs text-muted-foreground">
                        {record.name}
                      </span>
                      {record.in_zone ? null : <Badge variant="info">Other zone</Badge>}
                    </div>
                    <p className="font-mono text-xs break-all text-foreground">{record.value}</p>
                  </div>
                  <CopyButton value={record.value} label={`Copy ${record.record_type} record`} />
                </div>
              );
            })}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
