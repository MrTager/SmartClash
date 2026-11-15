import { expect, test } from 'vitest'

import apple from '@/assets/image/test/apple.svg?raw'

import { migrateDefaultTests, openai } from './test-defaults'

test('仅迁移旧默认 Apple 项并保留 UID、自定义项和顺序', () => {
  const original = { uid: 'existing', name: 'Apple', url: 'https://www.apple.com', icon: apple }
  const custom = { ...original, uid: 'custom', url: 'https://www.apple.com/custom' }
  const customIcon = { ...original, uid: 'custom-icon', icon: '<svg/>' }
  const result = migrateDefaultTests([original, custom, customIcon])
  expect(result[0]).toEqual({ uid: 'existing', name: 'OpenAI', url: 'https://api.openai.com/v1/models', icon: openai })
  expect(result[1]).toBe(custom)
  expect(result[2]).toBe(customIcon)
  expect(migrateDefaultTests(result)).toEqual(result)
  expect(migrateDefaultTests([])).toEqual([])
})
