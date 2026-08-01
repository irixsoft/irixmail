import * as React from "react";
import {
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  cn,
} from "@irixmail/shared";

import {
  WEEKDAYS,
  presetFromRule,
  recurrenceSummary,
  ruleForPreset,
  type RecurrencePreset,
} from "./event-form";
import type { RecurrenceFrequency, RecurrenceRule } from "./types";

const PRESETS: { id: RecurrencePreset; label: string }[] = [
  { id: "none", label: "Does not repeat" },
  { id: "daily", label: "Daily" },
  { id: "weekly", label: "Weekly" },
  { id: "monthly", label: "Monthly" },
  { id: "yearly", label: "Yearly" },
  { id: "custom", label: "Custom…" },
];

const FREQUENCIES: { id: RecurrenceFrequency; label: string }[] = [
  { id: "daily", label: "days" },
  { id: "weekly", label: "weeks" },
  { id: "monthly", label: "months" },
  { id: "yearly", label: "years" },
];

export function RecurrenceEditor({
  rule,
  startDate,
  onChange,
}: {
  rule: RecurrenceRule | null;
  startDate: string;
  onChange: (rule: RecurrenceRule | null) => void;
}) {
  const [forceCustom, setForceCustom] = React.useState(() => presetFromRule(rule) === "custom");
  const preset: RecurrencePreset = rule && forceCustom ? "custom" : presetFromRule(rule);
  const patch = (next: Partial<RecurrenceRule>) => {
    if (!rule) return;
    onChange({ ...rule, ...next });
  };
  const ends = rule?.count !== null && rule?.count !== undefined ? "count" : rule?.until ? "until" : "never";

  return (
    <div className="space-y-2">
      <Label className="text-xs text-muted-foreground">Repeat</Label>
      <Select
        value={preset}
        onValueChange={(value) => {
          const next = value as RecurrencePreset;
          setForceCustom(next === "custom");
          if (next === "custom") {
            if (!rule) onChange(ruleForPreset("weekly", startDate));
            return;
          }
          onChange(ruleForPreset(next, startDate));
        }}
      >
        <SelectTrigger className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {PRESETS.map((entry) => (
            <SelectItem key={entry.id} value={entry.id}>
              {entry.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      {preset === "custom" && rule ? (
        <div className="space-y-3 rounded-md border bg-muted/30 p-2.5">
          <div className="flex items-center gap-2">
            <span className="text-[13px] text-muted-foreground">Every</span>
            <Input
              type="number"
              min={1}
              max={99}
              value={rule.interval}
              onChange={(event) => patch({ interval: Math.max(1, Number(event.target.value) || 1) })}
              className="h-8 w-16 font-mono"
            />
            <Select value={rule.frequency} onValueChange={(value) => patch({ frequency: value as RecurrenceFrequency })}>
              <SelectTrigger className="h-8 flex-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {FREQUENCIES.map((entry) => (
                  <SelectItem key={entry.id} value={entry.id}>
                    {entry.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {rule.frequency === "weekly" ? (
            <div className="flex flex-wrap gap-1">
              {WEEKDAYS.map((day) => {
                const active = rule.byDay.includes(day.id);
                return (
                  <button
                    key={day.id}
                    type="button"
                    aria-pressed={active}
                    onClick={() =>
                      patch({
                        byDay: active ? rule.byDay.filter((entry) => entry !== day.id) : [...rule.byDay, day.id],
                      })
                    }
                    className={cn(
                      "size-8 rounded-full font-mono text-[11px] transition-colors",
                      active
                        ? "bg-primary text-primary-foreground"
                        : "bg-card text-muted-foreground hover:bg-accent hover:text-foreground",
                    )}
                  >
                    {day.label.slice(0, 2)}
                  </button>
                );
              })}
            </div>
          ) : null}

          <div className="flex items-center gap-2">
            <Select
              value={ends}
              onValueChange={(value) => {
                if (value === "never") patch({ count: null, until: null });
                if (value === "count") patch({ count: 10, until: null });
                if (value === "until") patch({ count: null, until: `${startDate}T00:00:00` });
              }}
            >
              <SelectTrigger className="h-8 w-32">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="never">Ends never</SelectItem>
                <SelectItem value="count">Ends after</SelectItem>
                <SelectItem value="until">Ends on</SelectItem>
              </SelectContent>
            </Select>
            {ends === "count" ? (
              <Input
                type="number"
                min={1}
                max={999}
                value={rule.count ?? 1}
                onChange={(event) => patch({ count: Math.max(1, Number(event.target.value) || 1) })}
                className="h-8 w-20 font-mono"
              />
            ) : null}
            {ends === "until" ? (
              <Input
                type="date"
                value={(rule.until ?? "").slice(0, 10)}
                onChange={(event) => patch({ until: `${event.target.value}T00:00:00` })}
                className="h-8 flex-1 font-mono"
              />
            ) : null}
          </div>
        </div>
      ) : null}

      <p className="font-mono text-[11px] text-muted-foreground">{recurrenceSummary(rule)}</p>
    </div>
  );
}
