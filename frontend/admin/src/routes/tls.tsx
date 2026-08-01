import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  ApiError,
  Badge,
  Button,
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
  ErrorState,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Skeleton,
  Textarea,
  toast,
  useAuth,
} from "@irixmail/shared";
import { RefreshCw, ShieldCheck, Upload } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { formatDateTime } from "@/lib/format";

const uploadSchema = z.object({
  certificate: z
    .string()
    .min(1, "Paste the certificate")
    .regex(/BEGIN CERTIFICATE/, "Must be a PEM-encoded certificate"),
  privateKey: z
    .string()
    .min(1, "Paste the private key")
    .regex(/PRIVATE KEY/, "Must be a PEM-encoded private key"),
});
type UploadValues = z.infer<typeof uploadSchema>;

function UploadCertCard() {
  const { client } = useAuth();
  const queryClient = useQueryClient();

  const form = useForm<UploadValues>({
    resolver: zodResolver(uploadSchema),
    defaultValues: { certificate: "", privateKey: "" },
  });

  const upload = useMutation({
    mutationFn: (body: UploadValues) =>
      client.post("/api/tls/upload", { certificate: body.certificate, privateKey: body.privateKey }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tls"] });
      toast.success("Certificate uploaded");
      form.reset();
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 400) {
        toast.error("The certificate or key was not valid");
      } else {
        toast.error("Could not upload the certificate");
      }
    },
  });

  const onSubmit = form.handleSubmit((values) => upload.mutate(values));

  return (
    <Card className="mt-6">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Upload className="size-4 text-primary" />
          Upload a custom certificate
        </CardTitle>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={onSubmit} className="grid gap-4" noValidate>
            <FormField
              control={form.control}
              name="certificate"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Certificate (PEM)</FormLabel>
                  <FormControl>
                    <Textarea
                      rows={5}
                      placeholder="-----BEGIN CERTIFICATE-----"
                      className="font-mono text-xs"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="privateKey"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Private key (PEM)</FormLabel>
                  <FormControl>
                    <Textarea
                      rows={5}
                      placeholder="-----BEGIN PRIVATE KEY-----"
                      className="font-mono text-xs"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="flex justify-end">
              <Button type="submit" loading={upload.isPending}>
                Upload certificate
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}

interface TlsStatus {
  status: string;
  issuer: string | null;
  sans: string[];
  expiresAt: number | null;
}

function statusVariant(status: string): "success" | "warning" | "muted" {
  switch (status) {
    case "valid":
    case "active":
    case "acme":
      return "success";
    case "self-signed":
    case "reissuing":
      return "warning";
    default:
      return "muted";
  }
}

function statusLabel(status: string): string {
  return status === "acme" ? "Let's Encrypt" : status;
}

export function TlsPage() {
  const { client } = useAuth();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["tls"],
    queryFn: ({ signal }) => client.get<TlsStatus>("/api/tls", { signal }),
  });

  const reissue = useMutation({
    mutationFn: () => client.post("/api/tls/reissue"),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["tls"] });
      toast.success("Certificate reissue requested");
    },
    onError: () => toast.error("Could not request a reissue"),
  });

  return (
    <div className="mx-auto max-w-2xl">
      <PageHeader title="TLS" description="Certificate status and management" />

      {query.isError ? (
        <ErrorState description="Could not load certificate status." onRetry={() => query.refetch()} />
      ) : query.isLoading || !query.data ? (
        <Skeleton className="h-48 w-full" />
      ) : (
        <Card>
          <CardHeader className="flex-row items-center justify-between">
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="size-4 text-primary" />
              Certificate
            </CardTitle>
            <Badge variant={statusVariant(query.data.status)}>{statusLabel(query.data.status)}</Badge>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="flex items-center justify-between border-t pt-4">
              <span className="text-muted-foreground">Issuer</span>
              <span className="font-mono">{query.data.issuer ?? "—"}</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-muted-foreground">Expires</span>
              <span className="font-mono">{formatDateTime(query.data.expiresAt)}</span>
            </div>
            <div className="space-y-2">
              <span className="text-muted-foreground">Subject alternative names</span>
              {query.data.sans.length === 0 ? (
                <p className="font-mono text-xs text-muted-foreground">None</p>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {query.data.sans.map((san) => (
                    <Badge key={san} variant="outline" className="font-mono">
                      {san}
                    </Badge>
                  ))}
                </div>
              )}
            </div>
          </CardContent>
          <CardFooter className="justify-end border-t pt-4">
            <Button variant="outline" onClick={() => reissue.mutate()} loading={reissue.isPending}>
              <RefreshCw className="size-4" />
              Reissue via ACME
            </Button>
          </CardFooter>
        </Card>
      )}

      <UploadCertCard />
    </div>
  );
}
