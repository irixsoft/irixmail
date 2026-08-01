import * as React from "react";
import { LoaderCircle } from "lucide-react";

import { cn } from "../../lib/utils";

export interface SpinnerProps extends React.ComponentProps<typeof LoaderCircle> {
  label?: string;
}

function Spinner({ className, label = "Loading", ...props }: SpinnerProps) {
  return (
    <LoaderCircle
      role="status"
      aria-label={label}
      className={cn("size-4 animate-spin text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Spinner };
