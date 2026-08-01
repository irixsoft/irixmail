import * as React from "react";
import { ChevronLeft, ChevronRight, MoreHorizontal } from "lucide-react";

import { cn } from "../../lib/utils";
import { Button } from "./button";

export interface PaginationProps extends Omit<React.ComponentProps<"nav">, "onChange"> {
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  siblingCount?: number;
}

function range(start: number, end: number): number[] {
  const out: number[] = [];
  for (let i = start; i <= end; i += 1) out.push(i);
  return out;
}

function getPages(
  page: number,
  pageCount: number,
  siblingCount: number,
): Array<number | "ellipsis"> {
  const left = Math.max(2, page - siblingCount);
  const right = Math.min(pageCount - 1, page + siblingCount);
  const pages: Array<number | "ellipsis"> = [1];
  if (left > 2) pages.push("ellipsis");
  pages.push(...range(left, right));
  if (right < pageCount - 1) pages.push("ellipsis");
  pages.push(pageCount);
  return pages;
}

function Pagination({
  page,
  pageCount,
  onPageChange,
  siblingCount = 1,
  className,
  ...props
}: PaginationProps) {
  if (pageCount <= 1) return null;
  const pages = getPages(page, pageCount, siblingCount);

  return (
    <nav
      role="navigation"
      aria-label="Pagination"
      className={cn("flex items-center justify-center gap-1", className)}
      {...props}
    >
      <Button
        variant="ghost"
        size="icon"
        disabled={page <= 1}
        onClick={() => onPageChange(page - 1)}
        aria-label="Previous page"
      >
        <ChevronLeft />
      </Button>
      {pages.map((entry, index) =>
        entry === "ellipsis" ? (
          <span
            key={`ellipsis-${index}`}
            className="flex size-9 items-center justify-center text-muted-foreground"
          >
            <MoreHorizontal className="size-4" />
          </span>
        ) : (
          <Button
            key={entry}
            variant={entry === page ? "default" : "ghost"}
            size="icon"
            aria-current={entry === page ? "page" : undefined}
            onClick={() => onPageChange(entry)}
          >
            {entry}
          </Button>
        ),
      )}
      <Button
        variant="ghost"
        size="icon"
        disabled={page >= pageCount}
        onClick={() => onPageChange(page + 1)}
        aria-label="Next page"
      >
        <ChevronRight />
      </Button>
    </nav>
  );
}

export { Pagination };
