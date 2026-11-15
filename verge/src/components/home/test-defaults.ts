import apple from '@/assets/image/test/apple.svg?raw'

export const openai = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" rx="12" fill="#10a37f"/><path d="M24 10 36 17v14L24 38 12 31V17Z" fill="none" stroke="white" stroke-width="3"/><path d="m12 17 24 14M24 10v28M36 17 12 31" stroke="white" stroke-width="2"/></svg>'

export const migrateDefaultTests = (items: IVergeTestItem[]) => items.map((item) =>
  item.name === 'Apple' && item.url === 'https://www.apple.com' && item.icon === apple
    ? { ...item, name: 'OpenAI', url: 'https://api.openai.com/v1/models', icon: openai }
    : item,
)
