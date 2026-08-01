import * as React from "react";

import {
  formToPayload,
  validateContactForm,
  type AddressRow,
  type ContactFormValues,
  type LabeledRow,
} from "./contact-form-mapping";
import type { ContactPayload } from "./types";

type RowField = "emails" | "phones";

const blankRow = (): LabeledRow => ({ value: "", label: "home" });

export function useContactForm(initial: ContactFormValues) {
  const [values, setValues] = React.useState<ContactFormValues>(initial);
  const [submitted, setSubmitted] = React.useState(false);

  const set = <K extends keyof ContactFormValues>(key: K, value: ContactFormValues[K]) =>
    setValues((current) => ({ ...current, [key]: value }));

  const setRow = (field: RowField, index: number, patch: Partial<LabeledRow>) =>
    setValues((current) => ({
      ...current,
      [field]: current[field].map((row, position) => (position === index ? { ...row, ...patch } : row)),
    }));

  const addRow = (field: RowField) =>
    setValues((current) => ({ ...current, [field]: [...current[field], blankRow()] }));

  const removeRow = (field: RowField, index: number) =>
    setValues((current) => {
      const rows = current[field].filter((_, position) => position !== index);
      return { ...current, [field]: rows.length > 0 ? rows : [blankRow()] };
    });

  const setAddress = (patch: Partial<AddressRow>) =>
    setValues((current) => ({ ...current, address: { ...current.address, ...patch } }));

  const error = validateContactForm(values);

  return {
    values,
    set,
    setRow,
    addRow,
    removeRow,
    setAddress,
    error,
    showError: submitted ? error : null,
    markSubmitted: () => setSubmitted(true),
    payload: (): ContactPayload => formToPayload(values),
  };
}
