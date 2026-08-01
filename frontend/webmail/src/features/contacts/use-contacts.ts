import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useJmap, useJmapSession } from "@/lib/jmap";
import type { AddressBook, ContactCard, ContactPayload } from "./types";

export interface AddressBookSetPayload {
  create?: Record<string, { name: string; description?: string | null }>;
  update?: Record<string, { name?: string; description?: string | null }>;
  destroy?: string[];
}

export interface ContactSetPayload {
  create?: Record<string, ContactPayload>;
  update?: Record<string, Partial<ContactPayload>>;
  destroy?: string[];
}

export function assertSetResult(result: Record<string, unknown>) {
  for (const key of ["notCreated", "notUpdated", "notDestroyed"]) {
    const failures = result[key] as Record<string, { description?: string; type?: string }> | null | undefined;
    const first = failures ? Object.values(failures)[0] : undefined;
    if (first) throw new Error(first.description ?? first.type ?? "The server rejected the change");
  }
}

export function useAddressBooks() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["address-books", accountId],
    queryFn: () => jmap.call<{ list: AddressBook[] }>("AddressBook/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const list = [...(query.data?.list ?? [])].sort((a, b) => {
    if (Boolean(a.isDefault) !== Boolean(b.isDefault)) return a.isDefault ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
  const byId: Record<string, AddressBook> = {};
  for (const book of list) byId[book.id] = book;
  const defaultId = list.find((book) => book.isDefault)?.id ?? list[0]?.id ?? "";

  return { query, list, byId, defaultId, accountId };
}

export function useAddressBookMutation() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const client = useQueryClient();

  return useMutation({
    mutationFn: async (payload: AddressBookSetPayload) => {
      const result = await jmap.call<Record<string, unknown>>("AddressBook/set", { accountId, ...payload });
      assertSetResult(result);
      return result;
    },
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["address-books"] });
      void client.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

export function useContacts() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();

  const query = useQuery({
    queryKey: ["contacts", accountId, "all"],
    queryFn: () => jmap.call<{ list: ContactCard[] }>("ContactCard/get", { accountId, ids: null }),
    enabled: Boolean(accountId),
  });

  const list = query.data?.list ?? [];
  const byId: Record<string, ContactCard> = {};
  for (const card of list) byId[card.id] = card;
  const groups = list.filter((card) => card.kind === "group");
  const individuals = list.filter((card) => card.kind !== "group");

  return { query, list, byId, groups, individuals, accountId };
}

export function useContactMutation() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const client = useQueryClient();

  return useMutation({
    mutationFn: async (payload: ContactSetPayload) => {
      const result = await jmap.call<Record<string, unknown>>("ContactCard/set", { accountId, ...payload });
      assertSetResult(result);
      return result;
    },
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

const IMPORT_CHUNK = 50;

export function useContactImport() {
  const jmap = useJmap();
  const { accountId } = useJmapSession();
  const client = useQueryClient();

  return useMutation({
    mutationFn: async (cards: ContactPayload[]) => {
      let created = 0;
      for (let index = 0; index < cards.length; index += IMPORT_CHUNK) {
        const chunk = cards.slice(index, index + IMPORT_CHUNK);
        const create: Record<string, ContactPayload> = {};
        chunk.forEach((card, position) => {
          create[`c${index + position}`] = card;
        });
        const result = await jmap.call<Record<string, unknown>>("ContactCard/set", { accountId, create });
        assertSetResult(result);
        created += Object.keys((result["created"] as Record<string, unknown>) ?? {}).length;
      }
      return created;
    },
    onSettled: () => {
      void client.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}
