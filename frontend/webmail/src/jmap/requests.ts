import type { JmapMethodCall } from "@irixmail/shared";

export const LIST_PROPS = [
  "id",
  "threadId",
  "mailboxIds",
  "keywords",
  "from",
  "to",
  "subject",
  "receivedAt",
  "preview",
  "hasAttachment",
  "size",
];

export interface EmailListOptions {
  filter: Record<string, unknown>;
  position: number;
  limit: number;
}

export function emailListCalls(accountId: string, options: EmailListOptions): JmapMethodCall[] {
  const queryId = "q";
  return [
    [
      "Email/query",
      {
        accountId,
        filter: options.filter,
        sort: [{ property: "receivedAt", isAscending: false }],
        position: options.position,
        limit: options.limit,
        calculateTotal: true,
      },
      queryId,
    ],
    [
      "Email/get",
      {
        accountId,
        "#ids": { resultOf: queryId, name: "Email/query", path: "/ids" },
        properties: LIST_PROPS,
      },
      "g",
    ],
  ];
}

export interface ThreadCallOptions {
  properties?: string[];
  bodyProperties?: string[];
  fetchBodies?: boolean;
}

export function threadCalls(
  accountId: string,
  threadId: string,
  options: ThreadCallOptions = {},
): JmapMethodCall[] {
  const threadCallId = "t";
  const getArgs: Record<string, unknown> = {
    accountId,
    "#ids": { resultOf: threadCallId, name: "Thread/get", path: "/list/*/emailIds" },
    properties: options.properties ?? LIST_PROPS,
  };
  if (options.bodyProperties) getArgs["bodyProperties"] = options.bodyProperties;
  if (options.fetchBodies) {
    getArgs["fetchHTMLBodyValues"] = true;
    getArgs["fetchTextBodyValues"] = true;
  }
  return [
    ["Thread/get", { accountId, ids: [threadId] }, threadCallId],
    ["Email/get", getArgs, "g"],
  ];
}
