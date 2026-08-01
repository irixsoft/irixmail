// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import {
  EMAIL_SURFACE,
  blockExternalResources,
  buildSrcDoc,
  plainTextToHtml,
  resolveCids,
  sanitizeEmailHtml,
  splitQuote,
  unblockExternalResources,
} from "./sanitize";

describe("sanitizeEmailHtml", () => {
  it("strips scripts and event handlers", () => {
    const clean = sanitizeEmailHtml(`<p onclick="x()">hi</p><script>alert(1)</script>`);
    expect(clean).not.toContain("script");
    expect(clean).not.toContain("onclick");
    expect(clean).toContain("hi");
  });

  it("strips style svg math iframe and form elements", () => {
    const clean = sanitizeEmailHtml(
      `<style>p{}</style><svg></svg><math></math><iframe src="x"></iframe><form><input></form><p>ok</p>`,
    );
    for (const tag of ["<style", "<svg", "<math", "<iframe", "<form", "<input"]) {
      expect(clean).not.toContain(tag);
    }
    expect(clean).toContain("ok");
  });

  it("keeps safe links and cid images, drops javascript urls", () => {
    const clean = sanitizeEmailHtml(
      `<a href="https://x.example">a</a><a href="javascript:alert(1)">b</a><img src="cid:img1">`,
    );
    expect(clean).toContain(`href="https://x.example"`);
    expect(clean).not.toContain("javascript:");
    expect(clean).toContain(`src="cid:img1"`);
  });

  it("allows raster data images but not svg data uris", () => {
    const clean = sanitizeEmailHtml(
      `<img src="data:image/png;base64,AAAA"><img src="data:image/svg+xml,<svg/>">`,
    );
    expect(clean).toContain("data:image/png");
    expect(clean).not.toContain("data:image/svg");
  });
});

describe("external resource blocking", () => {
  it("swaps external images for a placeholder and stashes the original", () => {
    const { html, blockedCount } = blockExternalResources(`<img src="https://t.example/p.png">`);
    expect(blockedCount).toBe(1);
    expect(html).toContain(`data-blocked-src="https://t.example/p.png"`);
    expect(html).toContain(`src="data:image/gif`);
    expect(html).not.toContain(` src="https://t.example/p.png"`);
  });

  it("leaves cid and data images alone", () => {
    const { html, blockedCount } = blockExternalResources(
      `<img src="cid:a"><img src="data:image/png;base64,AA">`,
    );
    expect(blockedCount).toBe(0);
    expect(html).toContain(`src="cid:a"`);
  });

  it("blocks css url() backgrounds in inline styles", () => {
    const { html, blockedCount } = blockExternalResources(
      `<div style="background-image:url('https://t.example/bg.png')">x</div>`,
    );
    expect(blockedCount).toBe(1);
    expect(html).not.toContain("https://t.example/bg.png\")");
    expect(html).toContain("data-blocked-style");
  });

  it("round-trips through unblock", () => {
    const blocked = blockExternalResources(`<img src="https://t.example/p.png">`);
    const restored = unblockExternalResources(blocked.html);
    expect(restored).toContain(`src="https://t.example/p.png"`);
    expect(restored).not.toContain("data-blocked-src");
  });
});

describe("resolveCids", () => {
  it("rewrites cid sources to blob urls", () => {
    const html = resolveCids(`<img src="cid:one"><img src="cid:two">`, {
      one: "blob:http://x/1",
    });
    expect(html).toContain(`src="blob:http://x/1"`);
    expect(html).toContain(`src="cid:two"`);
  });
});

describe("plainTextToHtml", () => {
  it("escapes html and linkifies urls", () => {
    const html = plainTextToHtml("see <b> at https://x.example/page\nnext");
    expect(html).toContain("&lt;b&gt;");
    expect(html).toContain(`<a href="https://x.example/page"`);
    expect(html).toContain("<br");
  });
});

describe("splitQuote", () => {
  it("splits a gmail quote out of the body", () => {
    const { main, quote } = splitQuote(
      `<p>reply</p><div class="gmail_quote"><p>original</p></div>`,
    );
    expect(main).toContain("reply");
    expect(main).not.toContain("original");
    expect(quote).toContain("original");
  });

  it("splits blockquote type=cite", () => {
    const { quote } = splitQuote(`<p>r</p><blockquote type="cite">orig</blockquote>`);
    expect(quote).toContain("orig");
  });

  it("returns no quote when none found", () => {
    expect(splitQuote(`<p>just text</p>`).quote).toBeNull();
  });
});

describe("buildSrcDoc", () => {
  it("embeds a blocking csp by default and http sources when allowed", () => {
    const blocked = buildSrcDoc("<p>x</p>", { allowExternal: false });
    expect(blocked).toContain("Content-Security-Policy");
    expect(blocked).toContain("img-src data: blob: cid:");
    const open = buildSrcDoc("<p>x</p>", { allowExternal: true });
    expect(open).toContain("img-src data: blob: cid: http: https:");
  });

  it("paints a light surface by default", () => {
    const doc = buildSrcDoc("<p>x</p>", { allowExternal: false });
    expect(doc).toContain("color-scheme:light");
    expect(doc).toContain(`background:${EMAIL_SURFACE.light}`);
    expect(doc).toContain("color:#26292e");
  });

  it("paints a dark surface with legible text when dark is set", () => {
    const doc = buildSrcDoc("<p>x</p>", { allowExternal: false, dark: true });
    expect(doc).toContain("color-scheme:dark");
    expect(doc).toContain(`background:${EMAIL_SURFACE.dark}`);
    expect(doc).not.toContain("color:#26292e");
    expect(doc).toContain("color:#d8d5cf");
    expect(doc).toContain("a{color:#d9a760}");
  });
});
