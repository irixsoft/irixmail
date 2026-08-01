import * as React from "react";

import { cn } from "../../lib/utils";
import { Input } from "./input";
import { Label } from "./label";

export interface TextFieldProps extends React.ComponentProps<"input"> {
  label?: React.ReactNode;
  hint?: React.ReactNode;
  error?: React.ReactNode;
  containerClassName?: string;
}

function TextField({
  id,
  label,
  hint,
  error,
  className,
  containerClassName,
  ...props
}: TextFieldProps) {
  const generatedId = React.useId();
  const fieldId = id ?? generatedId;
  const hintId = `${fieldId}-hint`;
  const errorId = `${fieldId}-error`;
  const invalid = Boolean(error);

  return (
    <div className={cn("grid gap-1.5", containerClassName)}>
      {label ? <Label htmlFor={fieldId}>{label}</Label> : null}
      <Input
        id={fieldId}
        aria-invalid={invalid || undefined}
        aria-describedby={cn(hint ? hintId : null, invalid ? errorId : null) || undefined}
        className={className}
        {...props}
      />
      {hint && !invalid ? (
        <p id={hintId} className="text-xs text-muted-foreground">
          {hint}
        </p>
      ) : null}
      {invalid ? (
        <p id={errorId} className="text-xs font-medium text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  );
}

export { TextField };
