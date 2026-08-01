import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { Button, Card, CardContent, ConfirmDialog, toast, useAuth } from "@irixmail/shared";
import { ShieldOff, Trash2 } from "lucide-react";

export function AccountSecurityTab({
  accountId,
  accountLabel,
}: {
  accountId: string;
  accountLabel: string;
}) {
  const { client } = useAuth();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirmReset, setConfirmReset] = React.useState(false);
  const [confirmDelete, setConfirmDelete] = React.useState(false);

  const reset2fa = useMutation({
    mutationFn: () => client.post(`/api/accounts/${accountId}/reset-2fa`),
    onSuccess: () => {
      setConfirmReset(false);
      toast.success("Two-factor authentication reset");
    },
    onError: () => toast.error("Could not reset two-factor authentication"),
  });

  const remove = useMutation({
    mutationFn: () => client.delete(`/api/accounts/${accountId}`),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["accounts"] });
      toast.success("Account deleted");
      navigate("/accounts");
    },
    onError: () => toast.error("Could not delete the account"),
  });

  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="text-sm font-medium">Reset two-factor authentication</p>
            <p className="text-xs text-muted-foreground">
              Removes the user&apos;s TOTP enrollment so they can set it up again.
            </p>
          </div>
          <Button variant="outline" onClick={() => setConfirmReset(true)}>
            <ShieldOff className="size-4" />
            Reset 2FA
          </Button>
        </CardContent>
      </Card>

      <Card className="border-destructive/30">
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <p className="text-sm font-medium">Delete account</p>
            <p className="text-xs text-muted-foreground">
              Permanently removes the mailbox, aliases, and credentials.
            </p>
          </div>
          <Button variant="destructive" onClick={() => setConfirmDelete(true)}>
            <Trash2 className="size-4" />
            Delete
          </Button>
        </CardContent>
      </Card>

      <ConfirmDialog
        open={confirmReset}
        onOpenChange={setConfirmReset}
        title="Reset two-factor authentication"
        description="The account owner will need to enrol a new authenticator app."
        confirmLabel="Reset 2FA"
        closeOnConfirm={false}
        loading={reset2fa.isPending}
        onConfirm={() => reset2fa.mutate()}
      />

      <ConfirmDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title="Delete account"
        description={`This permanently removes ${accountLabel} and all of its mail.`}
        confirmLabel="Delete account"
        destructive
        closeOnConfirm={false}
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}
