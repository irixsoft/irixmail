import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Input, Label, Skeleton, Switch, Textarea, toast } from "@irixmail/shared";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { SettingsCard, SettingsRow } from "./section-card";

interface VacationResponse {
  id: string;
  isEnabled: boolean;
  fromDate: string | null;
  toDate: string | null;
  subject: string | null;
  textBody: string | null;
}

function toLocalInput(value: string | null): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const offset = date.getTimezoneOffset() * 60000;
  return new Date(date.getTime() - offset).toISOString().slice(0, 16);
}

function toIso(value: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

export function AutoReplySection() {
  const jmap = useJmap();
  const queryClient = useQueryClient();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["vacation", accountId],
    queryFn: () =>
      jmap.call<{ list: VacationResponse[] }>("VacationResponse/get", {
        accountId,
        ids: ["singleton"],
      }),
    enabled: Boolean(accountId),
  });

  const [enabled, setEnabled] = React.useState(false);
  const [subject, setSubject] = React.useState("");
  const [body, setBody] = React.useState("");
  const [fromDate, setFromDate] = React.useState("");
  const [toDate, setToDate] = React.useState("");
  const loaded = React.useRef(false);

  React.useEffect(() => {
    const current = query.data?.list[0];
    if (current && !loaded.current) {
      loaded.current = true;
      setEnabled(current.isEnabled);
      setSubject(current.subject ?? "");
      setBody(current.textBody ?? "");
      setFromDate(toLocalInput(current.fromDate));
      setToDate(toLocalInput(current.toDate));
    }
  }, [query.data]);

  const save = useMutation({
    mutationFn: () =>
      jmap.call("VacationResponse/set", {
        accountId,
        update: {
          singleton: {
            isEnabled: enabled,
            subject: subject || null,
            textBody: body || null,
            fromDate: toIso(fromDate),
            toDate: toIso(toDate),
          },
        },
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["vacation", accountId] });
      toast.success("Vacation responder saved");
    },
    onError: () => toast.error("Could not save the vacation responder"),
  });

  if (query.isLoading) {
    return <Skeleton className="h-80 w-full rounded-lg" />;
  }

  return (
    <div className="space-y-4">
      <SettingsCard>
        <SettingsRow
          label="Automatic replies"
          hint="Answer incoming mail while you are away."
          htmlFor="vacation-enabled"
        >
          <Switch
            id="vacation-enabled"
            checked={enabled}
            onCheckedChange={setEnabled}
            aria-label="Vacation responder enabled"
          />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Reply"
        description="Sent once per sender while the responder is active."
        bodyClassName="grid gap-4"
        className={enabled ? undefined : "opacity-60"}
        footer={
          <Button onClick={() => save.mutate()} loading={save.isPending}>
            Save
          </Button>
        }
      >
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-1.5">
            <Label htmlFor="vacation-from" className="text-[13px]">
              From
            </Label>
            <Input
              id="vacation-from"
              type="datetime-local"
              className="font-mono"
              value={fromDate}
              onChange={(event) => setFromDate(event.target.value)}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="vacation-to" className="text-[13px]">
              Until
            </Label>
            <Input
              id="vacation-to"
              type="datetime-local"
              className="font-mono"
              value={toDate}
              onChange={(event) => setToDate(event.target.value)}
            />
          </div>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="vacation-subject" className="text-[13px]">
            Subject
          </Label>
          <Input
            id="vacation-subject"
            value={subject}
            onChange={(event) => setSubject(event.target.value)}
            placeholder="Out of office"
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="vacation-body" className="text-[13px]">
            Message
          </Label>
          <Textarea
            id="vacation-body"
            value={body}
            onChange={(event) => setBody(event.target.value)}
            rows={6}
          />
        </div>
      </SettingsCard>
    </div>
  );
}
