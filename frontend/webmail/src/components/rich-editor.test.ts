import { describe, expect, it } from "vitest";
import { Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";

import { editorValue } from "./rich-editor";

describe("editorValue", () => {
  it("reads html and text from a live editor", () => {
    const editor = new Editor({ extensions: [StarterKit], content: "<p>hi</p>" });
    expect(editorValue(editor)).toEqual({ html: "<p>hi</p>", text: "hi" });
    editor.destroy();
  });

  it("returns null for a destroyed editor instead of crashing", () => {
    const editor = new Editor({ extensions: [StarterKit] });
    editor.destroy();
    expect(editorValue(editor)).toBeNull();
  });
});
