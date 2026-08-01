import * as React from "react";

import {
  addNaiveMinutes,
  deviceTimeZone,
  formToPayload,
  joinNaive,
  naiveDiffDays,
  naiveDiffMinutes,
  splitNaive,
  validateEventForm,
  type EventFormValues,
  type EventPayload,
} from "./event-form";

export function useEventForm(initial: EventFormValues) {
  const [values, setValues] = React.useState<EventFormValues>(initial);
  const [submitted, setSubmitted] = React.useState(false);

  const set = <K extends keyof EventFormValues>(key: K, value: EventFormValues[K]) =>
    setValues((current) => ({ ...current, [key]: value }));

  const setStartDate = (startDate: string) =>
    setValues((current) => {
      const delta = naiveDiffDays(current.startDate, startDate);
      if (!Number.isFinite(delta)) return { ...current, startDate };
      const shifted = splitNaive(addNaiveMinutes(joinNaive(current.endDate, "00:00"), delta * 1440)).date;
      return { ...current, startDate, endDate: shifted || current.endDate };
    });

  const setStartTime = (startTime: string) =>
    setValues((current) => {
      const duration = naiveDiffMinutes(
        joinNaive(current.startDate, current.startTime),
        joinNaive(current.endDate, current.endTime),
      );
      if (!Number.isFinite(duration)) return { ...current, startTime };
      const end = splitNaive(addNaiveMinutes(joinNaive(current.startDate, startTime), duration));
      return { ...current, startTime, endDate: end.date || current.endDate, endTime: end.time };
    });

  const error = validateEventForm(values);

  return {
    values,
    set,
    setStartDate,
    setStartTime,
    error,
    showError: submitted ? error : null,
    markSubmitted: () => setSubmitted(true),
    payload: (): EventPayload => formToPayload(values, deviceTimeZone()),
  };
}
