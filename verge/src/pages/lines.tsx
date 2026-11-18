import { Alert, Box, Button, Chip, Stack, Tab, Tabs, Typography } from '@mui/material'
import { useState } from 'react'
import { useNavigate } from 'react-router'

import { BasePage } from '@/components/base'
import { CurrentProxyCard } from '@/components/home/current-proxy-card'
import { routePolicy } from '@/components/smartclash/route-policy'
import { SmartClashPanel } from '@/components/smartclash/smartclash-panel'
import { useSmartClash } from '@/hooks/use-smartclash'
import { useProxiesData } from '@/providers/app-data-context'
import { showNotice } from '@/services/notice-service'

const LinesPage = () => {
  const navigate = useNavigate()
  const { data, patch } = useSmartClash()
  const { proxyView } = useProxiesData()
  const [pending, setPending] = useState(false)
  const automatic = !!data?.settings.enabled && !!data.settings.automatic && data.status !== 'manual'
  const carrier = proxyView?.groups.find((group) => group.name === 'SMARTCLASH')
  const unified = !!data?.settings.enabled && !!data.settings.smartRules

  const selectPolicy = async (value: boolean) => {
    if (!data || pending) return
    setPending(true)
    try {
      const result = await patch(routePolicy(data.settings, value))
      if (result.ok && result.value.error) showNotice.error(result.value.error)
      if (result.ok && !result.value.error && value && data.status === 'manual') {
        const { optimizeSmartClash } = await import('@/services/cmds')
        const { mutate } = await import('@/services/mutate')
        const resumed = await mutate(optimizeSmartClash, { id: 'smartclash-optimize', revalidate: [['smartclash']] })
        if (resumed.ok && resumed.value.error) showNotice.error(resumed.value.error)
      }
    } catch {
      // The mutation service reports command failures.
    } finally {
      setPending(false)
    }
  }

  return (
    <BasePage title="线路" contentStyle={{ padding: 16 }}>
      <Stack spacing={2}>
        <Box sx={{ p: 2.5, borderRadius: 3, bgcolor: 'action.hover' }}>
          <Typography variant="overline">当前生效线路</Typography>
          <Typography variant="h5" sx={{ overflowWrap: 'anywhere' }}>
            {unified ? (carrier?.now ?? '等待内核加载线路') : '按现有订阅规则分流'}
          </Typography>
          <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
            <Chip size="small" label={automatic ? '自动选择' : '手动固定 / 订阅策略'} />
            <Chip size="small" variant="outlined" label="切换选路不改变系统代理或 TUN" />
          </Stack>
        </Box>
        {!unified && <Alert severity="info">选择下方选路方式后，将使用统一线路组及国内直连规则。现有系统代理和 TUN 状态保持不变。</Alert>}
        {!unified && <Button disabled={!data || pending} onClick={() => void selectPolicy(false)}>启用统一线路并手动固定</Button>}
        <Tabs value={automatic ? 'auto' : 'manual'} onChange={(_, value: string) => void selectPolicy(value === 'auto')}>
          <Tab label="自动选择" value="auto" disabled={!data || pending} />
          <Tab label="手动固定" value="manual" disabled={!data || pending} />
        </Tabs>
        {automatic ? <SmartClashPanel /> : <>
          <Alert severity="info">手动固定时不自动切换。请选择 SMARTCLASH 组中的线路；恢复自动选择后由后台接管。</Alert>
          <CurrentProxyCard trafficOnly={unified} />
        </>}
        <Button onClick={() => navigate('/advanced-proxies')} sx={{ alignSelf: 'start' }}>高级策略组（修改前请确认规则是否引用该组）</Button>
      </Stack>
    </BasePage>
  )
}

export default LinesPage
