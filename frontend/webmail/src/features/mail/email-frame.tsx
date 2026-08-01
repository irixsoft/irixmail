import * as React from "react";

import { useIsDark } from "@/lib/theme";
import { EMAIL_BODY_MARGIN, EMAIL_SURFACE, buildSrcDoc } from "./sanitize";

const VERTICAL_MARGIN = EMAIL_BODY_MARGIN.block * 2;

export function EmailFrame({ html, allowExternal }: { html: string; allowExternal: boolean }) {
  const ref = React.useRef<HTMLIFrameElement>(null);
  const observer = React.useRef<ResizeObserver | null>(null);
  const [height, setHeight] = React.useState(80);
  const dark = useIsDark();
  const srcDoc = React.useMemo(
    () => buildSrcDoc(html, { allowExternal, dark }),
    [html, allowExternal, dark],
  );

  React.useEffect(() => () => observer.current?.disconnect(), []);

  const onLoad = () => {
    observer.current?.disconnect();
    observer.current = null;
    const doc = ref.current?.contentDocument;
    const body = doc?.body;
    if (!doc || !body) return;
    const measure = () => {
      const content = Math.max(body.scrollHeight + VERTICAL_MARGIN, doc.documentElement.scrollHeight);
      setHeight(Math.min(20000, Math.max(40, content)));
    };
    measure();
    const next = new ResizeObserver(measure);
    next.observe(body);
    observer.current = next;
  };

  return (
    <iframe
      ref={ref}
      title="Message body"
      sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
      srcDoc={srcDoc}
      onLoad={onLoad}
      style={{ height, background: dark ? EMAIL_SURFACE.dark : EMAIL_SURFACE.light }}
      className="w-full border-0"
    />
  );
}
