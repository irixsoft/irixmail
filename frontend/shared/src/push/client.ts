export interface PushEvent {
  id: string | null;
  event: string;
  data: string;
}

export type PushListener = (event: PushEvent) => void;

export interface PushClientOptions {
  baseUrl?: string;
  path?: string;
  getToken?: () => string | null;
  types?: string[];
  ping?: number;
  reconnectDelayMs?: number;
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

export class PushClient {
  private readonly baseUrl: string;
  private readonly path: string;
  private readonly getToken: () => string | null;
  private readonly types: string[] | undefined;
  private readonly ping: number;
  private readonly reconnectDelayMs: number;

  private readonly listeners = new Set<PushListener>();
  private controller: AbortController | null = null;
  private lastEventId: string | null = null;
  private closed = false;

  constructor(options: PushClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
    this.path = options.path ?? "/jmap/eventsource";
    this.getToken = options.getToken ?? (() => null);
    this.types = options.types;
    this.ping = options.ping ?? 0;
    this.reconnectDelayMs = options.reconnectDelayMs ?? 3000;
  }

  on(listener: PushListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  start(): void {
    if (this.controller) return;
    this.closed = false;
    void this.run();
  }

  close(): void {
    this.closed = true;
    this.controller?.abort();
    this.controller = null;
  }

  private buildUrl(): string {
    const params = new URLSearchParams();
    params.set("types", this.types && this.types.length > 0 ? this.types.join(",") : "*");
    params.set("closeafter", "no");
    params.set("ping", String(this.ping));
    return `${this.baseUrl}${this.path}?${params.toString()}`;
  }

  private async run(): Promise<void> {
    while (!this.closed) {
      this.controller = new AbortController();
      try {
        const headers: Record<string, string> = { Accept: "text/event-stream" };
        const token = this.getToken();
        if (token) headers["Authorization"] = `Bearer ${token}`;
        if (this.lastEventId) headers["Last-Event-ID"] = this.lastEventId;
        const response = await fetch(this.buildUrl(), {
          headers,
          signal: this.controller.signal,
        });
        if (!response.ok || !response.body) {
          throw new Error(`eventsource failed (${response.status})`);
        }
        await this.readStream(response.body);
      } catch {
        if (this.closed) return;
      }
      if (this.closed) return;
      await delay(this.reconnectDelayMs, this.controller.signal);
    }
  }

  private async readStream(body: ReadableStream<Uint8Array>): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      if (value) buffer += decoder.decode(value, { stream: true }).replace(/\r/g, "");
      let boundary = buffer.indexOf("\n\n");
      while (boundary !== -1) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        this.dispatch(frame);
        boundary = buffer.indexOf("\n\n");
      }
    }
  }

  private dispatch(frame: string): void {
    let event = "message";
    let data = "";
    let id: string | null = null;
    for (const line of frame.split("\n")) {
      if (!line || line.startsWith(":")) continue;
      const colon = line.indexOf(":");
      const field = colon === -1 ? line : line.slice(0, colon);
      let value = colon === -1 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "event") event = value;
      else if (field === "data") data += data ? `\n${value}` : value;
      else if (field === "id") id = value;
    }
    if (id !== null) this.lastEventId = id;
    const pushEvent: PushEvent = { id, event, data };
    for (const listener of this.listeners) listener(pushEvent);
  }
}
