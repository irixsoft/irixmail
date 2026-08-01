import { addDays, startOfWeek } from "./layout";
import { TimeGrid, type TimeGridProps } from "./time-grid";

type WeekViewProps = Omit<TimeGridProps, "days" | "showDayHeaders"> & { anchor: Date };

export function WeekView({ anchor, ...rest }: WeekViewProps) {
  const first = startOfWeek(anchor);
  const days = Array.from({ length: 7 }, (_, index) => addDays(first, index));
  return <TimeGrid days={days} showDayHeaders {...rest} />;
}
