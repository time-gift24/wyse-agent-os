export function startApprovalSubmission(
  approvalIds: ReadonlySet<string>,
  approvalId: string
): Set<string> {
  return new Set(approvalIds).add(approvalId)
}

export function finishApprovalSubmission(
  approvalIds: ReadonlySet<string>,
  approvalId: string
): Set<string> {
  const nextApprovalIds = new Set(approvalIds)
  nextApprovalIds.delete(approvalId)
  return nextApprovalIds
}
