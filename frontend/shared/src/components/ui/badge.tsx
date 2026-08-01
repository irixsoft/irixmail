import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "../../lib/utils";

const badgeVariants = cva(
  "inline-flex w-fit shrink-0 items-center justify-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap transition-colors [&>svg]:pointer-events-none [&>svg]:size-3",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground",
        secondary: "border-transparent bg-secondary text-secondary-foreground",
        outline: "text-foreground",
        destructive: "border-transparent bg-destructive text-destructive-foreground",
        success: "border-transparent bg-success/15 text-success",
        warning: "border-transparent bg-warning/15 text-warning",
        info: "border-transparent bg-info/15 text-info",
        muted: "border-transparent bg-muted text-muted-foreground",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

function Badge({
  className,
  variant,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "span";
  return (
    <Comp data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

const dotTone = cva("rounded-full", {
  variants: {
    tone: {
      neutral: "bg-muted-foreground",
      success: "bg-success",
      warning: "bg-warning",
      danger: "bg-destructive",
      info: "bg-info",
    },
  },
  defaultVariants: { tone: "neutral" },
});

interface StatusDotProps
  extends React.ComponentProps<"span">,
    VariantProps<typeof dotTone> {
  pulse?: boolean;
}

function StatusDot({ className, tone, pulse = false, ...props }: StatusDotProps) {
  return (
    <span
      data-slot="status-dot"
      className={cn("relative flex size-2 items-center justify-center", className)}
      {...props}
    >
      {pulse ? (
        <span className={cn("absolute inline-flex h-full w-full animate-ping opacity-60", dotTone({ tone }))} />
      ) : null}
      <span className={cn("relative inline-flex size-2", dotTone({ tone }))} />
    </span>
  );
}

export { Badge, badgeVariants, StatusDot };
