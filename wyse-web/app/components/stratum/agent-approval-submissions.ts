export type ApprovalDecision = "approve" | "reject"

export function startApprovalSubmission(
  submissions: ReadonlyMap<string, ApprovalDecision>,
  approvalId: string,
  decision: ApprovalDecision
): Map<string, ApprovalDecision> {
  return new Map(submissions).set(approvalId, decision)
}

export function finishApprovalSubmission(
  submissions: ReadonlyMap<string, ApprovalDecision>,
  approvalId: string
): Map<string, ApprovalDecision> {
  const nextSubmissions = new Map(submissions)
  nextSubmissions.delete(approvalId)
  return nextSubmissions
}
