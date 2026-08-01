import * as React from "react";

import { useContacts } from "@/features/contacts/use-contacts";
import { rankSuggestions, type Suggestion } from "./suggestions";

export function useContactSuggestions(
  query: string,
  exclude: string[],
  limit?: number,
): Suggestion[] {
  const { list } = useContacts();
  const excludeKey = exclude.join(" ").toLowerCase();

  return React.useMemo(
    () => rankSuggestions(list, query, excludeKey.split(" ").filter(Boolean), limit),
    [list, query, excludeKey, limit],
  );
}
