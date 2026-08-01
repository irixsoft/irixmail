import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Label, Skeleton, Textarea, toast } from "@irixmail/shared";

import { useJmap, useJmapSession } from "@/lib/jmap";
import type { Identity } from "@/lib/mail-types";
import { SettingsCard } from "./section-card";

export function AccountSection() {
  const jmap = useJmap();
  const queryClient = useQueryClient();
  const { accountId, session } = useJmapSession();

  const query = useQuery({
    queryKey: ["identities", accountId],
    queryFn: () => jmap.call<{ list: Identity[] }>("Identity/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const identity = query.data?.list[0];
  const [name, setName] = React.useState("");
  const [signature, setSignature] = React.useState("");
  const loaded = React.useRef(false);

  React.useEffect(() => {
    if (query.data && !loaded.current) {
      loaded.current = true;
      setName(identity?.name ?? session?.username ?? "");
      setSignature((identity as { textSignature?: string } | undefined)?.textSignature ?? "");
    }
  }, [query.data, identity, session]);

  const save = useMutation({
    mutationFn: () => {
      const patch = { name, textSignature: signature };
      if (identity?.id) {
        return jmap.call("Identity/set", { accountId, update: { [identity.id]: patch } });
      }
      return jmap.call("Identity/set", {
        accountId,
        create: { id0: { ...patch, email: session?.username } },
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["identities", accountId] });
      toast.success("Identity saved");
    },
    onError: () => toast.error("Could not save your identity"),
  });

  if (query.isLoading) {
    return <Skeleton className="h-64 w-full rounded-lg" />;
  }

  return (
    <SettingsCard
      title="Identity"
      description="How your name and signature appear on the mail you send."
      bodyClassName="grid gap-4"
      footer={
        <Button onClick={() => save.mutate()} loading={save.isPending}>
          Save
        </Button>
      }
    >
      <div className="grid gap-1.5">
        <Label htmlFor="identity-name" className="text-[13px]">
          Display name
        </Label>
        <Input id="identity-name" value={name} onChange={(event) => setName(event.target.value)} />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="identity-email" className="text-[13px]">
          Email
        </Label>
        <Input
          id="identity-email"
          value={identity?.email ?? session?.username ?? ""}
          readOnly
          className="font-mono text-muted-foreground"
        />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="identity-signature" className="text-[13px]">
          Signature
        </Label>
        <Textarea
          id="identity-signature"
          value={signature}
          onChange={(event) => setSignature(event.target.value)}
          rows={6}
          placeholder="— Sent from IRIXMAIL"
        />
        <p className="text-xs text-muted-foreground">Plain text, appended to the end of new messages.</p>
      </div>
    </SettingsCard>
  );
}
