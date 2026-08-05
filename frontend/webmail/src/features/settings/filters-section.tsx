import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Button,
  ConfirmDialog,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  EmptyState,
  ErrorState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  toast,
} from "@irixmail/shared";
import { Filter, Pencil, Plus, Trash2 } from "lucide-react";

import { useJmap, useJmapSession } from "@/lib/jmap";
import { mailboxLabel, type Mailbox } from "@/lib/mail-types";
import {
  emptyRule,
  isExternallyEdited,
  removeRule,
  ruleSummary,
  savePayload,
  scriptRules,
  upsertRule,
  type FilterRule,
  type SieveScript,
} from "@/jmap/filters";
import { SettingsCard } from "./section-card";

export function FiltersSection() {
  const jmap = useJmap();
  const queryClient = useQueryClient();
  const { accountId } = useJmapSession();
  const [editing, setEditing] = React.useState<FilterRule | null>(null);
  const [pendingDelete, setPendingDelete] = React.useState<FilterRule | null>(null);
  const [pendingReset, setPendingReset] = React.useState(false);

  const query = useQuery({
    queryKey: ["sieve", accountId],
    queryFn: () => jmap.call<{ list: SieveScript[] }>("SieveScript/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });
  const mailboxesQuery = useQuery({
    queryKey: ["mailboxes", accountId],
    queryFn: () => jmap.call<{ list: Mailbox[] }>("Mailbox/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const script = query.data?.list[0];
  const rules = scriptRules(script) ?? [];
  const mailboxes = mailboxesQuery.data?.list ?? [];

  const persist = useMutation({
    mutationFn: (next: FilterRule[]) =>
      jmap.call("SieveScript/set", savePayload(accountId ?? "", script?.id, next)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["sieve", accountId] }),
    onError: () => toast.error("Could not save the rule"),
  });

  const saveRule = (rule: FilterRule) => {
    persist.mutate(upsertRule(rules, rule), {
      onSuccess: () => {
        setEditing(null);
        toast.success("Rule saved");
      },
    });
  };

  const deleteRule = (rule: FilterRule) => {
    persist.mutate(removeRule(rules, rule.id), {
      onSuccess: () => {
        setPendingDelete(null);
        toast.success("Rule deleted");
      },
    });
  };

  if (query.isError) {
    return <ErrorState description="Could not load filter rules." onRetry={() => query.refetch()} />;
  }
  if (query.isLoading) {
    return <Skeleton className="h-56 w-full rounded-lg" />;
  }

  if (isExternallyEdited(script)) {
    return (
      <div className="space-y-4">
        <SettingsCard
          title="Edited outside the webmail"
          description="This filter script was changed by another Sieve editor. It still runs on incoming mail, but it can no longer be shown as rules."
        >
          <pre className="max-h-64 overflow-auto rounded-md bg-muted/50 p-3 font-mono text-xs whitespace-pre-wrap">
            {script?.source ?? ""}
          </pre>
          <div className="mt-3">
            <Button variant="outline" size="sm" onClick={() => setPendingReset(true)}>
              Reset rules
            </Button>
          </div>
        </SettingsCard>
        <ConfirmDialog
          open={pendingReset}
          onOpenChange={(open) => {
            if (!open) setPendingReset(false);
          }}
          title="Reset rules"
          description="Replace the edited script with an empty rule list? The current script is discarded."
          confirmLabel="Reset"
          destructive
          closeOnConfirm={false}
          loading={persist.isPending}
          onConfirm={() => {
            persist.mutate([], {
              onSuccess: () => {
                setPendingReset(false);
                toast.success("Rules reset");
              },
            });
          }}
        />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <SettingsCard
        title="Rules"
        description={`${rules.length} rule${rules.length === 1 ? "" : "s"} run in order on incoming mail.`}
        action={
          <Button size="sm" onClick={() => setEditing(emptyRule())}>
            <Plus className="size-4" />
            New rule
          </Button>
        }
        bodyClassName={rules.length === 0 ? "p-4" : "p-0"}
      >
        {rules.length === 0 ? (
          <EmptyState
            icon={Filter}
            title="No filter rules"
            description="Create a rule to sort incoming mail automatically."
            className="border-0 bg-transparent py-8"
          />
        ) : (
          <ul className="divide-y">
            {rules.map((rule) => (
              <li key={rule.id} className="flex items-center justify-between gap-3 px-4 py-3">
                <div className="min-w-0">
                  <p className="truncate text-[13px] font-medium">{rule.name || "Untitled rule"}</p>
                  <p className="truncate font-mono text-xs text-muted-foreground">
                    {ruleSummary(rule)}
                  </p>
                </div>
                <div className="flex items-center gap-1">
                  <Button variant="ghost" size="icon" aria-label="Edit" onClick={() => setEditing(rule)}>
                    <Pencil className="size-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="Delete"
                    onClick={() => setPendingDelete(rule)}
                  >
                    <Trash2 className="size-4 text-destructive" />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </SettingsCard>

      <RuleEditor
        rule={editing}
        mailboxes={mailboxes}
        saving={persist.isPending}
        onCancel={() => setEditing(null)}
        onSave={saveRule}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title="Delete rule"
        description={pendingDelete ? `Delete "${pendingDelete.name || "Untitled rule"}"?` : undefined}
        confirmLabel="Delete"
        destructive
        closeOnConfirm={false}
        loading={persist.isPending}
        onConfirm={() => {
          if (pendingDelete) deleteRule(pendingDelete);
        }}
      />
    </div>
  );
}

function RuleEditor({
  rule,
  mailboxes,
  saving,
  onCancel,
  onSave,
}: {
  rule: FilterRule | null;
  mailboxes: Mailbox[];
  saving: boolean;
  onCancel: () => void;
  onSave: (rule: FilterRule) => void;
}) {
  const [draft, setDraft] = React.useState<FilterRule | null>(rule);

  React.useEffect(() => {
    setDraft(rule);
  }, [rule]);

  if (!draft) return null;
  const update = (patch: Partial<FilterRule>) =>
    setDraft((prev) => (prev ? { ...prev, ...patch } : prev));

  return (
    <Dialog open={rule !== null} onOpenChange={(open) => (!open ? onCancel() : undefined)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Filter rule</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label className="text-[13px]">Name</Label>
            <Input value={draft.name} onChange={(event) => update({ name: event.target.value })} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Select
              value={draft.field}
              onValueChange={(value) => update({ field: value as FilterRule["field"] })}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="from">From</SelectItem>
                <SelectItem value="to">To</SelectItem>
                <SelectItem value="subject">Subject</SelectItem>
              </SelectContent>
            </Select>
            <Select
              value={draft.operator}
              onValueChange={(value) => update({ operator: value as FilterRule["operator"] })}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="contains">contains</SelectItem>
                <SelectItem value="is">is</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <Input
            value={draft.value}
            placeholder="value"
            className="font-mono"
            onChange={(event) => update({ value: event.target.value })}
          />
          <Select
            value={draft.action}
            onValueChange={(value) => update({ action: value as FilterRule["action"], target: "" })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fileinto">Move to folder</SelectItem>
              <SelectItem value="forward">Forward to</SelectItem>
              <SelectItem value="markRead">Mark as read</SelectItem>
              <SelectItem value="discard">Discard</SelectItem>
            </SelectContent>
          </Select>
          {draft.action === "fileinto" ? (
            <Select value={draft.target} onValueChange={(value) => update({ target: value })}>
              <SelectTrigger>
                <SelectValue placeholder="Choose a folder" />
              </SelectTrigger>
              <SelectContent>
                {mailboxes.map((mailbox) => (
                  <SelectItem key={mailbox.id} value={mailboxLabel(mailbox)}>
                    {mailboxLabel(mailbox)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : draft.action === "forward" ? (
            <Input
              value={draft.target}
              placeholder="forward@example.com"
              className="font-mono"
              onChange={(event) => update({ target: event.target.value })}
            />
          ) : null}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={() => onSave(draft)} loading={saving}>
            Save rule
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
