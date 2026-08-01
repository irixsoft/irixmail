import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Badge, Switch, toast } from "@irixmail/shared";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { disableWebPush, enableWebPush, pushStatus } from "@/pwa/web-push";
import { SettingsCard, SettingsRow } from "./section-card";

export function NotificationsSection() {
  const jmap = useJmap();
  const { session, accountId } = useJmapSession();
  const queryClient = useQueryClient();

  const status = useQuery({
    queryKey: ["push-status", accountId],
    enabled: Boolean(session && accountId),
    queryFn: () => pushStatus(jmap, session!, accountId!),
  });

  const toggle = useMutation({
    mutationFn: async (enable: boolean) => {
      if (!session || !accountId) return;
      if (enable) await enableWebPush(jmap, session, accountId);
      else await disableWebPush(jmap, accountId);
    },
    onSuccess: (_data, enable) => {
      toast.success(enable ? "Push notifications on — verifying…" : "Push notifications off");
      void queryClient.invalidateQueries({ queryKey: ["push-status"] });
    },
    onError: (error: Error) => toast.error(error.message || "Could not update push notifications"),
  });

  const value = status.data;
  const hint = !value
    ? "Checking…"
    : !value.supported
      ? "This browser does not support push notifications."
      : !value.keyAvailable
        ? "The server has no push key configured."
        : value.permission === "denied"
          ? "Notifications are blocked in the browser settings."
          : "Get notified about new mail even when the app is closed.";

  return (
    <SettingsCard>
      <SettingsRow label="Push notifications" hint={hint}>
        <div className="flex items-center gap-2">
          {value?.enabled ? (
            <Badge variant={value.verified ? "default" : "muted"} className="font-mono text-[10px]">
              {value.verified ? "verified" : "verifying"}
            </Badge>
          ) : null}
          <Switch
            checked={Boolean(value?.enabled)}
            disabled={
              !value || !value.supported || !value.keyAvailable || value.permission === "denied" || toggle.isPending
            }
            onCheckedChange={(checked) => toggle.mutate(checked)}
            aria-label="Push notifications"
          />
        </div>
      </SettingsRow>
    </SettingsCard>
  );
}
