// @vitest-environment jsdom
import { describe, expect, it } from "vitest";

import {
  attributionLine,
  quotedHtml,
  quotedHtmlFromText,
  quotedText,
  senderLabel,
  threadingHeaders,
} from "./quote";

describe("senderLabel", () => {
  it("renders name and address when a name is present", () => {
    expect(senderLabel([{ name: "Ada", email: "ada@example.com" }])).toBe("Ada <ada@example.com>");
    expect(senderLabel([{ email: "ada@example.com" }])).toBe("ada@example.com");
    expect(senderLabel(null)).toBe("the sender");
  });
});

describe("attributionLine", () => {
  it("is a single line naming the sender", () => {
    const line = attributionLine([{ email: "ada@example.com" }], "2026-01-02T03:04:05Z");
    expect(line.startsWith("On ")).toBe(true);
    expect(line.endsWith("ada@example.com wrote:")).toBe(true);
    expect(line.includes("\n")).toBe(false);
  });
});

describe("quotedHtml", () => {
  it("wraps sanitized html in a blockquote below the attribution", () => {
    const html = quotedHtml("On day, ada wrote:", "<p>hello<script>alert(1)</script></p>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("hello");
    expect(html).not.toContain("<script");
  });

  it("escapes the attribution line", () => {
    expect(quotedHtml("Ada <ada@example.com> wrote:", "<p>hi</p>")).toContain("&lt;ada@example.com&gt;");
  });
});

describe("quotedHtmlFromText", () => {
  it("escapes the quoted plain text", () => {
    const html = quotedHtmlFromText("line", "1 < 2 & 3");
    expect(html).toContain("1 &lt; 2 &amp; 3");
  });
});

describe("quotedText", () => {
  it("prefixes every original line", () => {
    expect(quotedText("line", "a\n\nb")).toBe("\n\nline\n> a\n>\n> b\n");
  });
});

describe("threadingHeaders", () => {
  const source = { messageId: ["<m1@x>"], references: ["<r1@x>", "<r2@x>"] };

  it("replies carry inReplyTo and extended references", () => {
    expect(threadingHeaders("reply", source)).toEqual({
      inReplyTo: ["<m1@x>"],
      references: ["<r1@x>", "<r2@x>", "<m1@x>"],
    });
    expect(threadingHeaders("replyAll", source)).toEqual({
      inReplyTo: ["<m1@x>"],
      references: ["<r1@x>", "<r2@x>", "<m1@x>"],
    });
  });

  it("forwards keep references but no inReplyTo", () => {
    expect(threadingHeaders("forward", source)).toEqual({
      references: ["<r1@x>", "<r2@x>", "<m1@x>"],
    });
  });

  it("a source without messageId yields nothing to thread on", () => {
    expect(threadingHeaders("reply", {})).toEqual({});
    expect(threadingHeaders("reply", null)).toEqual({});
    expect(threadingHeaders(undefined, source)).toEqual({});
  });
});
