import * as React from "react";
import { EditorContent, useEditor, useEditorState, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Underline from "@tiptap/extension-underline";
import { Button, Input, cn } from "@irixmail/shared";
import {
  Bold,
  Check,
  Italic,
  Link2,
  Link2Off,
  List,
  ListOrdered,
  RemoveFormatting,
  Underline as UnderlineIcon,
  X,
} from "lucide-react";

export interface RichEditorValue {
  html: string;
  text: string;
}

export interface RichEditorProps {
  initialHtml?: string;
  onChange: (value: RichEditorValue) => void;
}

// Safari can destroy the instance before effects flush; a destroyed editor has no schema.
export function editorValue(editor: Editor): RichEditorValue | null {
  if (editor.isDestroyed) return null;
  return { html: editor.getHTML(), text: editor.getText({ blockSeparator: "\n\n" }) };
}

const CONTENT_CLASS = cn(
  "min-h-[18rem] px-3 py-2.5 text-sm leading-relaxed outline-none",
  "[&_a]:text-primary [&_a]:underline [&_strong]:font-semibold",
  "[&_p]:my-0 [&_p+p]:mt-3",
  "[&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6 [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_li]:my-1",
  "[&_h1]:mt-4 [&_h1]:mb-2 [&_h1]:text-lg [&_h1]:font-semibold",
  "[&_h2]:mt-4 [&_h2]:mb-2 [&_h2]:text-base [&_h2]:font-semibold",
  "[&_h3]:mt-3 [&_h3]:mb-1.5 [&_h3]:text-sm [&_h3]:font-semibold",
  "[&_blockquote]:my-3 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground",
  "[&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[0.85em]",
  "[&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-3 [&_pre]:text-xs",
  "[&_pre_code]:bg-transparent [&_pre_code]:p-0",
  "[&_hr]:my-4 [&_hr]:border-border",
  "[&_img]:max-w-full",
);

function normalizeHref(value: string): string {
  const href = value.trim();
  if (/^(https?:|mailto:)/i.test(href)) return href;
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(href)) return `mailto:${href}`;
  return `https://${href}`;
}

interface ToolButtonProps {
  label: string;
  active?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}

function ToolButton({ label, active = false, onClick, children }: ToolButtonProps) {
  return (
    <Button
      type="button"
      variant={active ? "secondary" : "ghost"}
      size="icon"
      className="size-8"
      aria-label={label}
      aria-pressed={active}
      title={label}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onClick}
    >
      {children}
    </Button>
  );
}

function Divider() {
  return <span aria-hidden="true" className="mx-1 h-5 w-px shrink-0 bg-border" />;
}

export function RichEditor({ initialHtml = "", onChange }: RichEditorProps) {
  const onChangeRef = React.useRef(onChange);
  onChangeRef.current = onChange;

  const [linkOpen, setLinkOpen] = React.useState(false);
  const [linkValue, setLinkValue] = React.useState("");
  const linkInputRef = React.useRef<HTMLInputElement>(null);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({ link: false, underline: false }),
      Underline,
      Link.configure({ openOnClick: false, autolink: true }),
    ],
    content: initialHtml,
    editorProps: {
      attributes: {
        class: CONTENT_CLASS,
        "aria-label": "Message body",
        "aria-multiline": "true",
      },
    },
    onUpdate: ({ editor: instance }) => {
      onChangeRef.current({
        html: instance.getHTML(),
        text: instance.getText({ blockSeparator: "\n\n" }),
      });
    },
  });

  React.useEffect(() => {
    const value = editorValue(editor);
    if (value) onChangeRef.current(value);
  }, [editor]);

  const seeded = React.useRef(initialHtml);
  React.useEffect(() => {
    if (initialHtml === seeded.current) return;
    seeded.current = initialHtml;
    if (!editor.isDestroyed) editor.commands.setContent(initialHtml);
  }, [editor, initialHtml]);

  const marks = useEditorState({
    editor,
    selector: ({ editor: instance }) => ({
      bold: instance.isActive("bold"),
      italic: instance.isActive("italic"),
      underline: instance.isActive("underline"),
      bulletList: instance.isActive("bulletList"),
      orderedList: instance.isActive("orderedList"),
      link: instance.isActive("link"),
    }),
  });

  const openLink = () => {
    setLinkValue((editor.getAttributes("link").href as string | undefined) ?? "");
    setLinkOpen(true);
    requestAnimationFrame(() => linkInputRef.current?.select());
  };

  const closeLink = () => {
    setLinkOpen(false);
    editor.commands.focus();
  };

  const applyLink = () => {
    const href = linkValue.trim();
    const chain = editor.chain().focus().extendMarkRange("link");
    if (href) chain.setLink({ href: normalizeHref(href) }).run();
    else chain.unsetLink().run();
    setLinkOpen(false);
  };

  const removeLink = () => {
    editor.chain().focus().extendMarkRange("link").unsetLink().run();
    setLinkOpen(false);
  };

  return (
    <div
      className={cn(
        "rounded-md border border-input bg-transparent shadow-xs outline-none",
        "transition-[color,box-shadow] motion-reduce:transition-none",
        "focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/40",
      )}
    >
      <div
        role="toolbar"
        aria-label="Formatting"
        className="flex flex-wrap items-center gap-0.5 border-b border-input px-1.5 py-1"
      >
        <ToolButton
          label="Bold"
          active={marks.bold}
          onClick={() => editor.chain().focus().toggleBold().run()}
        >
          <Bold className="size-4" />
        </ToolButton>
        <ToolButton
          label="Italic"
          active={marks.italic}
          onClick={() => editor.chain().focus().toggleItalic().run()}
        >
          <Italic className="size-4" />
        </ToolButton>
        <ToolButton
          label="Underline"
          active={marks.underline}
          onClick={() => editor.chain().focus().toggleUnderline().run()}
        >
          <UnderlineIcon className="size-4" />
        </ToolButton>

        <Divider />

        <ToolButton
          label="Bulleted list"
          active={marks.bulletList}
          onClick={() => editor.chain().focus().toggleBulletList().run()}
        >
          <List className="size-4" />
        </ToolButton>
        <ToolButton
          label="Numbered list"
          active={marks.orderedList}
          onClick={() => editor.chain().focus().toggleOrderedList().run()}
        >
          <ListOrdered className="size-4" />
        </ToolButton>

        <Divider />

        <ToolButton label="Add link" active={marks.link || linkOpen} onClick={openLink}>
          <Link2 className="size-4" />
        </ToolButton>
        <ToolButton
          label="Clear formatting"
          onClick={() => editor.chain().focus().unsetAllMarks().clearNodes().run()}
        >
          <RemoveFormatting className="size-4" />
        </ToolButton>
      </div>

      {linkOpen ? (
        <div className="flex items-center gap-1.5 border-b border-input px-1.5 py-1.5">
          <Input
            ref={linkInputRef}
            autoFocus
            value={linkValue}
            onChange={(event) => setLinkValue(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                applyLink();
              }
              if (event.key === "Escape") {
                event.preventDefault();
                closeLink();
              }
            }}
            placeholder="example.com"
            aria-label="Link address"
            className="h-8 font-mono text-xs shadow-none"
          />
          <Button
            type="button"
            variant="secondary"
            size="icon"
            className="size-8"
            aria-label="Apply link"
            title="Apply link"
            onClick={applyLink}
          >
            <Check className="size-4" />
          </Button>
          {marks.link ? (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-8"
              aria-label="Remove link"
              title="Remove link"
              onClick={removeLink}
            >
              <Link2Off className="size-4" />
            </Button>
          ) : null}
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-8"
            aria-label="Cancel"
            title="Cancel"
            onClick={closeLink}
          >
            <X className="size-4" />
          </Button>
        </div>
      ) : null}

      <EditorContent editor={editor} />
    </div>
  );
}
