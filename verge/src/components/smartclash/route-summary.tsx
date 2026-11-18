import { Button, Paper, Stack, Typography } from '@mui/material'
import { useNavigate } from 'react-router'

import { useSmartClash } from '@/hooks/use-smartclash'
import { useProxiesData } from '@/providers/app-data-context'

export const RouteSummary = () => {
  const { data } = useSmartClash()
  const { proxyView } = useProxiesData()
  const navigate = useNavigate()
  const unified = data?.settings.enabled && data.settings.smartRules
  const carrier = proxyView?.groups.find((group) => group.name === 'SMARTCLASH')
  return <Paper variant="outlined" sx={{ p: 2.5, mb: 2, borderRadius: 3 }}>
    <Typography variant="overline">当前线路 · 所有页面共享</Typography>
    <Typography variant="h5" sx={{ overflowWrap: 'anywhere' }}>{unified ? (carrier?.now ?? '等待线路加载') : '按订阅规则分流'}</Typography>
    <Typography color="text.secondary" sx={{ my: 1 }}>{!data ? '正在读取选路状态' : data.status === 'manual' ? '手动接管 · 自动选择已暂停' : data.settings.enabled && data.settings.automatic ? '自动选择 · 后台维护线路' : '手动固定 / 订阅策略'} · 实际出口位置待验证</Typography>
    {data?.error && <Typography color="warning.main">{data.error}</Typography>}
    <Stack direction="row" spacing={1}>
      <Button variant="outlined" onClick={() => navigate('/proxies')}>管理线路</Button>
      <Button onClick={() => navigate('/unlock')}>检查网络</Button>
    </Stack>
  </Paper>
}
