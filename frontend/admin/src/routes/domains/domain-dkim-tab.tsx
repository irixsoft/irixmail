import { useQuery } from "@tanstack/react-query";
import { Badge, Card, CardContent, EmptyState, ErrorState, Skeleton, useAuth } from "@irixmail/shared";
import { KeyRound } from "lucide-react";

import { CopyButton } from "@/components/copy-button";
import type { DnsRecord } from "@/lib/types";

interface DkimResponse {
  domain: string;
  keyIds: string[];
  records: DnsRecord[];
}

export function DomainDkimTab({ domainId }: { domainId: string }) {
  const { client } = useAuth();

  const query = useQuery({
    queryKey: ["domain", domainId, "dkim"],
    queryFn: ({ signal }) => client.get<DkimResponse>(`/api/domains/${domainId}/dkim`, { signal }),
  });

  if (query.isError) {
    return <ErrorState description="Could not load DKIM keys." onRetry={() => query.refetch()} />;
  }
  if (query.isLoading || !query.data) {
    return <Skeleton className="h-40 w-full" />;
  }

  const { keyIds, records } = query.data;

  return (
    <div className="space-y-4">
      <Card className="py-0">
        <CardContent className="space-y-3 p-5">
          <p className="font-mono text-[10px] tracking-wider text-muted-foreground uppercase">
            Signing keys
          </p>
          {keyIds.length === 0 ? (
            <p className="text-sm text-muted-foreground">No DKIM keys generated yet.</p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {keyIds.map((keyId) => (
                <Badge key={keyId} variant="outline" className="font-mono">
                  <KeyRound className="size-3" />
                  {keyId}
                </Badge>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {records.length === 0 ? (
        <EmptyState
          icon={KeyRound}
          title="No DKIM records to publish yet"
          description="The DKIM DNS record also appears on the DNS tab once keys are ready."
        />
      ) : (
        <Card className="py-0">
          <CardContent className="divide-y p-0">
            {records.map((record) => (
              <div key={`${record.name}:${record.record_type}`} className="flex items-start gap-3 p-4">
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex items-center gap-2">
                    <Badge variant="outline" className="font-mono">
                      {record.record_type}
                    </Badge>
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {record.name}
                    </span>
                  </div>
                  <p className="font-mono text-xs break-all text-foreground">{record.value}</p>
                </div>
                <CopyButton value={record.value} label="Copy DKIM record" />
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
