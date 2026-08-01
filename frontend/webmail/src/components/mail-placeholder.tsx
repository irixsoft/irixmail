import { EmptyState } from "@irixmail/shared";
import { Mail } from "lucide-react";

export function MailPlaceholder({ title, description }: { title: string; description?: string }) {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <EmptyState icon={Mail} title={title} description={description} />
    </div>
  );
}
