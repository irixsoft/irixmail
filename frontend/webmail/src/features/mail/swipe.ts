export const SWIPE_THRESHOLD = 96;

export type SwipeAction = "archive" | "menu";

export function resolveSwipe(offsetX: number): SwipeAction | null {
  if (offsetX > SWIPE_THRESHOLD) return "archive";
  if (offsetX < -SWIPE_THRESHOLD) return "menu";
  return null;
}
