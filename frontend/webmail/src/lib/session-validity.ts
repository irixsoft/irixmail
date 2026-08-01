import { JmapError } from "@irixmail/shared";

export function sessionStillValid(error: unknown): boolean {
  if (error instanceof JmapError && (error.status === 401 || error.status === 403)) return false;
  return true;
}
