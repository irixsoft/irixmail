import { beforeEach, describe, expect, it } from "vitest";
import {
  applicationServerKey,
  classifyPushPayload,
  deviceClientIdFor,
  requiresVisibleNotification,
  stateChangeNotice,
  subscriptionCreateArgs,
  urlBase64ToUint8Array,
} from "./push";

beforeEach(() => localStorage.clear());

describe("applicationServerKey", () => {
  it("reads the webpush capability", () => {
    const session = {
      capabilities: { "urn:irixmail:webpush": { applicationServerKey: "BKey123" } },
    };
    expect(applicationServerKey(session as never)).toBe("BKey123");
  });

  it("returns null when the capability is absent", () => {
    expect(applicationServerKey({ capabilities: {} } as never)).toBeNull();
  });
});

describe("urlBase64ToUint8Array", () => {
  it("decodes url-safe base64 with padding", () => {
    const bytes = urlBase64ToUint8Array("AQID");
    expect([...bytes]).toEqual([1, 2, 3]);
    expect([...urlBase64ToUint8Array("_-8")]).toEqual([255, 239]);
  });
});

describe("deviceClientIdFor", () => {
  it("is stable per account and distinct across accounts", () => {
    const first = deviceClientIdFor("1");
    expect(deviceClientIdFor("1")).toBe(first);
    expect(deviceClientIdFor("2")).not.toBe(first);
    expect(first.length).toBeGreaterThanOrEqual(16);
  });
});

describe("subscriptionCreateArgs", () => {
  it("builds the create payload from a browser subscription", () => {
    const args = subscriptionCreateArgs("device-1", {
      endpoint: "https://push.example/x",
      keys: { p256dh: "pkey", auth: "akey" },
    });
    expect(args).toEqual({
      deviceClientId: "device-1",
      url: "https://push.example/x",
      keys: { p256dh: "pkey", auth: "akey" },
    });
  });
});

const IPHONE_UA =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const MAC_SAFARI_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";
const MAC_CHROME_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
const MAC_FIREFOX_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:140.0) Gecko/20100101 Firefox/140.0";
const WINDOWS_CHROME_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";

describe("requiresVisibleNotification", () => {
  it("is true on iphone and ipad", () => {
    expect(requiresVisibleNotification(IPHONE_UA)).toBe(true);
    expect(requiresVisibleNotification(IPHONE_UA.replace(/iPhone/g, "iPad"))).toBe(true);
  });

  it("is true on mac safari but not mac chrome or firefox", () => {
    expect(requiresVisibleNotification(MAC_SAFARI_UA)).toBe(true);
    expect(requiresVisibleNotification(MAC_CHROME_UA)).toBe(false);
    expect(requiresVisibleNotification(MAC_FIREFOX_UA)).toBe(false);
  });

  it("is false elsewhere", () => {
    expect(requiresVisibleNotification(WINDOWS_CHROME_UA)).toBe(false);
    expect(requiresVisibleNotification("")).toBe(false);
  });
});

describe("stateChangeNotice", () => {
  const mailChange = { "1": { Email: "7", Mailbox: "3" } };
  const mailboxOnlyChange = { "1": { Mailbox: "3" } };

  it("announces new mail when no window is visible", () => {
    expect(stateChangeNotice(mailChange, false, WINDOWS_CHROME_UA)).toEqual({
      title: "New mail",
      body: "You have new mail.",
    });
  });

  it("stays silent for a visible window on platforms that allow it", () => {
    expect(stateChangeNotice(mailChange, true, WINDOWS_CHROME_UA)).toBeNull();
  });

  it("always announces on webkit platforms even when visible", () => {
    expect(stateChangeNotice(mailChange, true, IPHONE_UA)).toEqual({
      title: "New mail",
      body: "You have new mail.",
    });
  });

  it("announces a generic update on webkit when the change has no mail", () => {
    expect(stateChangeNotice(mailboxOnlyChange, false, IPHONE_UA)).toEqual({
      title: "Mailbox updated",
      body: "Your mailbox changed on another device.",
    });
  });

  it("stays silent for non-mail changes elsewhere", () => {
    expect(stateChangeNotice(mailboxOnlyChange, false, WINDOWS_CHROME_UA)).toBeNull();
  });
});

describe("classifyPushPayload", () => {
  it("classifies a verification payload", () => {
    expect(
      classifyPushPayload({
        "@type": "PushVerification",
        pushSubscriptionId: "5",
        verificationCode: "abc",
      }),
    ).toEqual({ kind: "verification", subscriptionId: "5", code: "abc" });
  });

  it("classifies a state change", () => {
    expect(
      classifyPushPayload({ "@type": "StateChange", changed: { "1": { Email: "7" } } }),
    ).toEqual({ kind: "stateChange", changed: { "1": { Email: "7" } } });
  });

  it("rejects junk", () => {
    expect(classifyPushPayload({ hello: 1 })).toBeNull();
    expect(classifyPushPayload(null)).toBeNull();
  });
});
