export function shouldAutoScroll(
  hasUserIntent: boolean,
  prefersReducedMotion: boolean
): boolean {
  return !hasUserIntent && !prefersReducedMotion
}
