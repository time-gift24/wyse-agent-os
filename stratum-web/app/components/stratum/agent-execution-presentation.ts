const RESOURCE_KEYS = ["file_path", "path", "filename", "file"] as const

export function humanizeToolName(
  name: string | null,
  fallback: string
): string {
  if (!name) return fallback

  return name
    .replace(/([a-z\d])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .toLocaleLowerCase()
}

export function approvalResource(argumentsValue: unknown): string | null {
  if (typeof argumentsValue !== "object" || argumentsValue === null) return null

  const values = argumentsValue as Record<string, unknown>
  for (const key of RESOURCE_KEYS) {
    const value = values[key]
    if (typeof value === "string" && value.trim() !== "") {
      return value.trim()
    }
  }
  return null
}
