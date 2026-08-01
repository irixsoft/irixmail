import { EmptyState } from "@irixmail/shared";
import { Construction } from "lucide-react";

export function PagePlaceholder({ title }: { title: string }) {
  return <EmptyState icon={Construction} title={title} description="This section is coming together." />;
}
