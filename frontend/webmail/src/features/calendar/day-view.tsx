import { startOfDay } from "./layout";
import { TimeGrid, type TimeGridProps } from "./time-grid";

type DayViewProps = Omit<TimeGridProps, "days" | "showDayHeaders"> & { anchor: Date };

export function DayView({ anchor, ...rest }: DayViewProps) {
  return <TimeGrid days={[startOfDay(anchor)]} showDayHeaders={false} {...rest} />;
}
