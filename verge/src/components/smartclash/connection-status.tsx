import { Box, Chip, Stack, Typography } from '@mui/material'
import { useTranslation } from 'react-i18next'

import ProxyControlSwitches from '@/components/shared/proxy-control-switches'
import { useSystemProxyState } from '@/hooks/use-system-proxy-state'
import { useSystemState } from '@/hooks/use-system-state'
import { useVerge } from '@/hooks/use-verge'
import { showNotice } from '@/services/notice-service'

export const ConnectionStatus = () => {
  const { t } = useTranslation()
  const { indicator } = useSystemProxyState()
  const { verge } = useVerge()
  const { runningMode, isTunModeAvailable, isLoading, runState } = useSystemState()
  const running = !isLoading && runningMode !== 'NotRunning'
  const tun = running && isTunModeAvailable && !!verge?.enable_tun_mode
  const label = isLoading ? '正在读取连接状态' : runState.opInFlight ? '正在应用连接设置' : !running ? '内核未运行' : indicator || tun ? '已接管流量 · 出网待验证' : '内核运行中 · 尚未接管流量'

  return <Box sx={{ p: 2, borderRadius: 3, border: 1, borderColor: 'divider', mb: 2 }}>
    <Stack direction="row" sx={{ justifyContent: 'space-between', gap: 1, flexWrap: 'wrap' }}>
      <Typography variant="h6">连接控制</Typography>
      <Chip size="small" label={label} color={running && (indicator || tun) ? 'success' : 'default'} />
    </Stack>
    <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>全局接管设置，与自动选择、手动固定无关。开启接管不代表已验证出网。</Typography>
    <Box sx={{ display: 'grid', gridTemplateColumns: { xs: '1fr', sm: '1fr 1fr' } }}>
      <ProxyControlSwitches onError={showNotice.error} />
      <ProxyControlSwitches label={t('settings.sections.system.toggles.tunMode')} onError={showNotice.error} />
    </Box>
  </Box>
}
