export type UiError = {
  code?: string
  stage?: string
  operationId?: string | null
  retryable?: boolean
  safeMessage: string
  technicalDetail?: string | null
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function parseAppError(raw: unknown): UiError {
  if (isObject(raw) && typeof raw.safeMessage === 'string') {
    return {
      code: typeof raw.code === 'string' ? raw.code : undefined,
      stage: typeof raw.stage === 'string' ? raw.stage : undefined,
      operationId: typeof raw.operationId === 'string' ? raw.operationId : null,
      retryable: typeof raw.retryable === 'boolean' ? raw.retryable : undefined,
      safeMessage: raw.safeMessage,
      technicalDetail: typeof raw.technicalDetail === 'string' ? raw.technicalDetail : null,
    }
  }
  if (raw instanceof Error) {
    return { safeMessage: raw.message || '操作失败' }
  }
  if (typeof raw === 'string') {
    try {
      const parsed = JSON.parse(raw)
      if (isObject(parsed) && typeof parsed.safeMessage === 'string') {
        return parseAppError(parsed)
      }
    } catch {
      // Plain string error message.
    }
    return { safeMessage: raw }
  }
  return { safeMessage: String(raw || '未知错误') }
}

export function errorSummary(raw: unknown): string {
  return parseAppError(raw).safeMessage
}
