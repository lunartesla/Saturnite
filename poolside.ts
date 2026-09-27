/**
 * Poolside transformer.
 *
 * Poolside serves an OpenAI-compatible chat-completions endpoint
 * (models like `laguna-m.1`). All models natively support extended
 * thinking and customizable reasoning effort, with full control over
 * thinking token allocation.
 *
 * Poolside models use RLCEF (Reinforcement Learning from Code Execution
 * Feedback) training, making reasoning/thinking a core capability for
 * software engineering tasks.
 *
 * Base URL: https://inference.poolside.ai/v1 (a dedicated inference
 * subdomain - NOT the poolside.ai marketing site, which 404s any API
 * path). Confirmed against https://docs.poolside.ai. Override via
 * POOLSIDE_BASE_URL.
 * Models are addressed with a `poolside/` namespace prefix, e.g. `poolside/laguna-m.1`.
 * Auth: Bearer token. Poolside issues non-standard `sky_vrID`-shaped
 * secret keys in addition to more conventional ones, so no key-prefix
 * format check is enforced (see api_key_manager.ts / ProviderLoginFlow).
 */

import type { Transformer, TransformContext } from './base.js'
import type { OpenAIChatRequest } from './shared_types.js'

// Poolside effort budgets aligned with opencode's terracode scale.
// Poolside natively supports massive reasoning budgets for software engineering.
const POOLSIDE_EFFORT_BUDGETS: Record<string, number> = {
  low: 4096,
  medium: 10000,
  high: 24000,
  xhigh: 48000,
  // Terracode/max level - Poolside models support massive reasoning budgets
  max: 200000,
  terracode: 200000,
}

export const poolsideTransformer: Transformer = {
  id: 'poolside',
  displayName: 'Poolside',
  defaultBaseUrl: 'https://inference.poolside.ai/v1',

  supportsStrictMode: () => false,

  clampMaxTokens(requested: number): number {
    return requested
  },

  transformRequest(body: OpenAIChatRequest, ctx: TransformContext): OpenAIChatRequest {
    // Poolside addresses its models with a `poolside/` namespace prefix
    // on the wire (e.g. `poolside/laguna-m.1`), even though users type
    // the bare model id (`laguna-m.1`) at the CLI.
    if (!ctx.model.startsWith('poolside/')) {
      body.model = `poolside/${ctx.model}`
    }

    // Poolside supports both reasoning_effort and thinking parameters.
    // When reasoning_effort is provided via the lane's thinking param,
    // we inject both formats for maximum compatibility.
    if (ctx.reasoningEffort) {
      body.reasoning_effort = ctx.reasoningEffort

      // Inject thinking token budget - Poolside natively accepts this
      const budget = POOLSIDE_EFFORT_BUDGETS[ctx.reasoningEffort]
      if (budget !== undefined) {
        body.thinking = {
          type: 'enabled',
          budget_tokens: budget,
        }
      }
    }
    return body
  },

  schemaDropList(): Set<string> {
    return new Set(['$schema', '$id', '$ref', '$comment'])
  },

  contextExceededMarkers(): string[] {
    return ['context length', 'context_length_exceeded', 'prompt is too long', 'token limit', 'too long']
  },

  preferredEditFormat(_model: string): 'apply_patch' | 'edit_block' | 'str_replace' {
    // Poolside models excel at code - use edit_block for good compatibility
    return 'edit_block'
  },

  smallFastModel(_model: string): string | null {
    // Poolside's laguna-m.1 is already efficient - no separate small model needed
    return null
  },

  cacheControlMode(): 'none' | 'passthrough' | 'last-only' {
    // Poolside may support caching in future - for now strip to be safe
    return 'none'
  },
}