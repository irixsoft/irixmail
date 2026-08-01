import { Spinner } from "@irixmail/shared";

export function FullPageLoader({ label = "Loading" }: { label?: string }) {
  return (
    <div className="bg-grid flex min-h-svh items-center justify-center bg-background">
      <div className="flex flex-col items-center gap-3 text-muted-foreground">
        <Spinner className="size-6 text-primary" />
        <p className="font-mono text-xs tracking-widest uppercase">{label}</p>
      </div>
    </div>
  );
}
