import { cn } from "@irixmail/shared";

export function Brand({ className }: { className?: string }) {
  return (
    <div className={cn("flex items-center gap-2 font-mono text-sm font-semibold tracking-tight", className)}>
      <span className="inline-flex size-2 rounded-full bg-primary shadow-[0_0_10px] shadow-primary/60" />
      <span className="text-foreground">
        IRIX<span className="text-primary">MAIL</span>
      </span>
    </div>
  );
}
