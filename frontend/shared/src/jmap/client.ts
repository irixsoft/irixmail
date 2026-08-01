export interface JmapAccount {
  name: string;
  isPersonal: boolean;
  isReadOnly: boolean;
  accountCapabilities: Record<string, unknown>;
}

export interface JmapSession {
  capabilities: Record<string, unknown>;
  accounts: Record<string, JmapAccount>;
  primaryAccounts: Record<string, string>;
  username: string;
  apiUrl: string;
  downloadUrl: string;
  uploadUrl: string;
  eventSourceUrl: string;
  state: string;
}

export type JmapMethodCall = [string, Record<string, unknown>, string];

export interface JmapRequest {
  using: string[];
  methodCalls: JmapMethodCall[];
  createdIds?: Record<string, string>;
}

export interface JmapResponse {
  methodResponses: JmapMethodCall[];
  sessionState: string;
  createdIds?: Record<string, string>;
}

export interface JmapBlobUploadResult {
  accountId: string;
  blobId: string;
  type: string;
  size: number;
}

export class JmapError extends Error {
  readonly detail: unknown;
  readonly status: number | undefined;

  constructor(message: string, detail?: unknown, status?: number) {
    super(message);
    this.name = "JmapError";
    this.detail = detail;
    this.status = status;
  }
}

export interface JmapClientOptions {
  baseUrl?: string;
  getToken?: () => string | null;
  onUnauthorized?: () => void;
}

export interface CallOptions {
  callId?: string;
  using?: string[];
  signal?: AbortSignal;
}

const DEFAULT_USING = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"];

export class JmapClient {
  private readonly baseUrl: string;
  private readonly getToken: () => string | null;
  private readonly onUnauthorized: (() => void) | undefined;
  private cachedSession: JmapSession | null = null;

  constructor(options: JmapClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
    this.getToken = options.getToken ?? (() => null);
    this.onUnauthorized = options.onUnauthorized;
  }

  private authHeaders(extra?: Record<string, string>): Record<string, string> {
    const headers: Record<string, string> = { Accept: "application/json", ...extra };
    const token = this.getToken();
    if (token) headers["Authorization"] = `Bearer ${token}`;
    return headers;
  }

  async session(force = false): Promise<JmapSession> {
    if (this.cachedSession && !force) return this.cachedSession;
    const session = await this.fetchJson<JmapSession>("GET", "/jmap/session");
    this.cachedSession = session;
    return session;
  }

  async request(
    methodCalls: JmapMethodCall[],
    using: string[] = DEFAULT_USING,
    signal?: AbortSignal,
  ): Promise<JmapResponse> {
    return this.fetchJson<JmapResponse>("POST", "/jmap/", { using, methodCalls }, signal);
  }

  async call<T = Record<string, unknown>>(
    name: string,
    args: Record<string, unknown>,
    options: CallOptions = {},
  ): Promise<T> {
    const callId = options.callId ?? "c0";
    const response = await this.request([[name, args, callId]], options.using ?? DEFAULT_USING, options.signal);
    const match = response.methodResponses.find((entry) => entry[2] === callId);
    if (!match) throw new JmapError(`no response for method call "${name}"`, response);
    const [responseName, responseArgs] = match;
    if (responseName === "error") {
      throw new JmapError(String(responseArgs["type"] ?? "error"), responseArgs);
    }
    return responseArgs as T;
  }

  async uploadBlob(accountId: string, blob: Blob, signal?: AbortSignal): Promise<JmapBlobUploadResult> {
    const url = `${this.baseUrl}/jmap/upload/${encodeURIComponent(accountId)}/`;
    const response = await fetch(url, {
      method: "POST",
      headers: this.authHeaders({ "Content-Type": blob.type || "application/octet-stream" }),
      body: blob,
      signal,
    });
    if (response.status === 401) this.onUnauthorized?.();
    if (!response.ok) {
      throw new JmapError(`blob upload failed (${response.status})`, await response.text(), response.status);
    }
    return (await response.json()) as JmapBlobUploadResult;
  }

  downloadUrl(accountId: string, blobId: string, name: string): string {
    const template = this.cachedSession?.downloadUrl ?? "/jmap/download/{accountId}/{blobId}/{name}";
    const path = template
      .replace("{accountId}", encodeURIComponent(accountId))
      .replace("{blobId}", encodeURIComponent(blobId))
      .replace("{name}", encodeURIComponent(name));
    return template.startsWith("http") ? path : `${this.baseUrl}${path}`;
  }

  private async fetchJson<T>(
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<T> {
    const headers = this.authHeaders(
      body !== undefined ? { "Content-Type": "application/json" } : undefined,
    );
    const response = await fetch(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
      signal,
    });
    if (response.status === 401) this.onUnauthorized?.();
    const text = await response.text();
    const data = text ? (JSON.parse(text) as unknown) : undefined;
    if (!response.ok) {
      const envelope = data as { error?: { message?: string }; detail?: string } | undefined;
      const message =
        envelope?.error?.message ?? envelope?.detail ?? `JMAP request failed (${response.status})`;
      throw new JmapError(message, data, response.status);
    }
    return data as T;
  }
}
