import * as React from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  ApiError,
  Button,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  useAuth,
} from "@irixmail/shared";

import { Brand } from "@/components/brand";

const credentialsSchema = z.object({
  username: z.email("Enter a valid email address"),
  password: z.string().min(1, "Password is required"),
});
type CredentialsValues = z.infer<typeof credentialsSchema>;

const totpSchema = z.object({
  code: z.string().regex(/^\d{6}$/, "Enter the six-digit code"),
});
type TotpValues = z.infer<typeof totpSchema>;

export function LoginPage() {
  const { login, verifyTotp, status } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/";
  const [stage, setStage] = React.useState<"credentials" | "totp">("credentials");

  React.useEffect(() => {
    if (status === "authenticated") navigate(from, { replace: true });
  }, [status, from, navigate]);

  const credentialsForm = useForm<CredentialsValues>({
    resolver: zodResolver(credentialsSchema),
    defaultValues: { username: "", password: "" },
  });
  const totpForm = useForm<TotpValues>({
    resolver: zodResolver(totpSchema),
    defaultValues: { code: "" },
  });

  const onCredentials = credentialsForm.handleSubmit(async (values) => {
    try {
      const outcome = await login(values.username, values.password);
      if (outcome.status === "totp_required") setStage("totp");
    } catch (error) {
      const message = error instanceof ApiError ? error.message : "Could not sign in";
      credentialsForm.setError("password", { message });
    }
  });

  const onTotp = totpForm.handleSubmit(async (values) => {
    try {
      await verifyTotp(values.code);
    } catch (error) {
      const message = error instanceof ApiError ? error.message : "Invalid or expired code";
      totpForm.setError("code", { message });
    }
  });

  return (
    <div className="bg-grid relative flex min-h-svh items-center justify-center bg-background p-4">
      <div className="pointer-events-none absolute inset-x-0 top-0 h-64 bg-gradient-to-b from-primary/10 to-transparent" />
      <div className="relative w-full max-w-sm">
        <div className="mb-6 flex flex-col items-center gap-3 text-center">
          <Brand className="text-base" />
          <div>
            <h1 className="text-lg font-semibold">Webmail</h1>
            <p className="text-sm text-muted-foreground">Sign in to your mailbox</p>
          </div>
        </div>

        <div className="rounded-lg border bg-card p-6 shadow-sm">
          {stage === "credentials" ? (
            <Form {...credentialsForm}>
              <form onSubmit={onCredentials} className="grid gap-4" noValidate>
                <FormField
                  control={credentialsForm.control}
                  name="username"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Email</FormLabel>
                      <FormControl>
                        <Input
                          type="email"
                          autoComplete="username"
                          placeholder="you@example.com"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={credentialsForm.control}
                  name="password"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Password</FormLabel>
                      <FormControl>
                        <Input type="password" autoComplete="current-password" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button type="submit" loading={credentialsForm.formState.isSubmitting}>
                  Sign in
                </Button>
              </form>
            </Form>
          ) : (
            <Form {...totpForm}>
              <form onSubmit={onTotp} className="grid gap-4" noValidate>
                <FormField
                  control={totpForm.control}
                  name="code"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Authentication code</FormLabel>
                      <FormControl>
                        <Input
                          inputMode="numeric"
                          autoComplete="one-time-code"
                          maxLength={6}
                          placeholder="000000"
                          className="text-center font-mono text-base tracking-[0.5em]"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button type="submit" loading={totpForm.formState.isSubmitting}>
                  Verify
                </Button>
                <Button type="button" variant="ghost" size="sm" onClick={() => setStage("credentials")}>
                  Back
                </Button>
              </form>
            </Form>
          )}
        </div>
      </div>
    </div>
  );
}
