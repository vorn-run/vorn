import type { WorkflowTemplate } from '../../packages/shared/src/types'
import seed from '../../packages/core/crates/connectors/data/template-seed.json'

/** The templates offered before any catalog is fetched, as vornd bundles them. */
export const TEMPLATE_SEED = seed as unknown as WorkflowTemplate[]
