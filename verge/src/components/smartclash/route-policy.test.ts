import { expect, test } from 'vitest'

import type { SmartClashSettings } from '@/services/smartclash-types'

import { routePolicy } from './route-policy'

const settings: SmartClashSettings = {
  enabled: false, automatic: false, smartRules: false, mode: 'ai',
  preferNonHongKong: true, chainEnabled: true, entryNodes: ['entry'],
  exitNodes: ['exit'], globeAnimation: false, sourceProfiles: [], exploreMultihop: false,
}

test('switching policies preserves the carrier, candidate chain and user preferences', () => {
  const automatic = routePolicy(settings, true)
  const manual = routePolicy(automatic, false)
  expect(automatic).toEqual({ ...settings, enabled: true, automatic: true, smartRules: true })
  expect(manual).toEqual({ ...automatic, automatic: false })
  expect(settings.enabled).toBe(false)
  expect(manual).not.toHaveProperty('enable_tun_mode')
  expect(manual).not.toHaveProperty('enable_system_proxy')
})
