import * as React from "react";
import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  toast,
} from "@irixmail/shared";
import { Upload } from "lucide-react";

import { planImport } from "./import-plan";
import { parseVcf, parsedToPayload, type ParsedCard } from "./vcard";
import { useContactImport, useContacts } from "./use-contacts";
import type { AddressBook } from "./types";

export interface ImportDialogProps {
  open: boolean;
  books: AddressBook[];
  defaultBookId: string;
  onOpenChange: (open: boolean) => void;
}

export function ImportDialog({ open, books, defaultBookId, onOpenChange }: ImportDialogProps) {
  const { list } = useContacts();
  const importer = useContactImport();
  const [parsed, setParsed] = React.useState<ParsedCard[] | null>(null);
  const [filename, setFilename] = React.useState("");
  const [skipDuplicates, setSkipDuplicates] = React.useState(true);
  const [bookId, setBookId] = React.useState(defaultBookId);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) {
      setParsed(null);
      setFilename("");
      setSkipDuplicates(true);
    }
  }, [open]);

  React.useEffect(() => {
    if (defaultBookId) setBookId((current) => current || defaultBookId);
  }, [defaultBookId]);

  const plan = parsed ? planImport(parsed, list) : null;
  const selection = plan ? (skipDuplicates ? plan.fresh : [...plan.fresh, ...plan.duplicates]) : [];

  const readFile = async (file: File) => {
    try {
      const cards = parseVcf(await file.text());
      if (cards.length === 0) {
        toast.error("No contacts found in that file");
        return;
      }
      setFilename(file.name);
      setParsed(cards);
    } catch {
      toast.error("Could not read that file");
    }
  };

  const run = () => {
    if (!bookId) {
      toast.error("Pick an address book");
      return;
    }
    importer.mutate(
      selection.map((card) => parsedToPayload(card, bookId)),
      {
        onSuccess: (created) => {
          toast.success(`Imported ${created} contact${created === 1 ? "" : "s"}`);
          onOpenChange(false);
        },
        onError: (error) => toast.error(error.message),
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Import contacts</DialogTitle>
          <DialogDescription>Pick a vCard (.vcf) file to add its contacts.</DialogDescription>
        </DialogHeader>

        <input
          ref={inputRef}
          type="file"
          accept=".vcf,text/vcard,text/x-vcard"
          className="sr-only"
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void readFile(file);
          }}
        />

        <div className="space-y-3">
          <Button variant="secondary" className="w-full justify-start gap-2" onClick={() => inputRef.current?.click()}>
            <Upload className="size-4" /> {filename || "Choose a .vcf file"}
          </Button>

          {plan ? (
            <>
              <p className="text-sm text-muted-foreground">
                <span className="font-mono tabular-nums text-foreground">{parsed?.length ?? 0}</span> contacts in the
                file,{" "}
                <span className="font-mono tabular-nums text-foreground">{plan.duplicates.length}</span> already here.
              </p>

              <label className="flex items-center gap-2.5 text-sm">
                <Checkbox
                  checked={skipDuplicates}
                  onCheckedChange={(checked) => setSkipDuplicates(checked === true)}
                />
                Skip duplicates
              </label>

              <div className="space-y-1.5">
                <Label className="text-xs text-muted-foreground">Address book</Label>
                <Select value={bookId} onValueChange={setBookId}>
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Pick an address book" />
                  </SelectTrigger>
                  <SelectContent>
                    {books.map((book) => (
                      <SelectItem key={book.id} value={book.id}>
                        {book.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </>
          ) : null}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={importer.isPending}>
            Cancel
          </Button>
          <Button onClick={run} disabled={importer.isPending || selection.length === 0}>
            {importer.isPending ? <Spinner className="size-4" /> : null}
            Import {selection.length > 0 ? selection.length : ""}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
