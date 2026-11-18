import type { SmartClashSettings } from '@/services/smartclash-types'

// Both policies use the same traffic carrier; only selection ownership changes.
export const routePolicy = (settings: SmartClashSettings, automatic: boolean): SmartClashSettings => ({
  ...settings,
  enabled: true,
  smartRules: true,
  automatic,
})
