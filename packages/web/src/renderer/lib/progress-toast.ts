import { toast } from '../components/Toast'

interface ProgressLabels {
  loading: string
  success: string
  error?: (err: unknown) => string
}

export async function withProgressToast<T>(
  labels: ProgressLabels,
  fn: () => Promise<T>
): Promise<T | undefined> {
  const id = toast.loading(labels.loading)
  try {
    const result = await fn()
    toast.update(id, labels.success, 'success')
    return result
  } catch (err) {
    const msg = labels.error ? labels.error(err) : err instanceof Error ? err.message : String(err)
    toast.update(id, msg || 'Operation failed', 'error')
    return undefined
  }
}
