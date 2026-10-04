import type { NativePipeline } from './native-core'

/**
 * Terminals whose output runs on a core thread, by session id.
 *
 * A module of its own because two others answer from it: the screen model,
 * which creates and frees a pipeline, and the scrollback, which lives inside
 * one. Either importing the other would be a cycle.
 */
const pipelines = new Map<string, NativePipeline>()

export function pipelineFor(id: string): NativePipeline | undefined {
  return pipelines.get(id)
}

export function holdPipeline(id: string, pipeline: NativePipeline): void {
  pipelines.set(id, pipeline)
}

export function releasePipeline(id: string): NativePipeline | undefined {
  const pipeline = pipelines.get(id)
  pipelines.delete(id)
  return pipeline
}

export function pipelineIds(): string[] {
  return [...pipelines.keys()]
}
