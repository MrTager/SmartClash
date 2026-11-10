import {
  getSmartClashState,
  optimizeSmartClash,
  patchSmartClashSettings,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { useQuery } from '@/services/query-client'
import type { SmartClashSettings } from '@/services/smartclash-types'

export const useSmartClash = () => {
  const query = useQuery({
    queryKey: ['smartclash'],
    queryFn: getSmartClashState,
    refetchInterval: 2000,
    refetchOnWindowFocus: true,
  })
  const patch = (settings: SmartClashSettings) =>
    mutate(() => patchSmartClashSettings(settings), {
      id: 'smartclash-settings',
      revalidate: [['smartclash']],
    })
  const optimize = () =>
    mutate(optimizeSmartClash, {
      id: 'smartclash-optimize',
      revalidate: [['smartclash']],
    })
  return { ...query, patch, optimize }
}
