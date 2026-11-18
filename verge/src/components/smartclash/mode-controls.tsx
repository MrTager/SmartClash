import { AutoModeRounded, LanRounded, PowerSettingsNewRounded } from '@mui/icons-material'
import { Button, Paper, Stack, Tooltip, Typography } from '@mui/material'
import { useLockFn } from 'ahooks'

import { useSmartClash } from '@/hooks/use-smartclash'
import { useVerge } from '@/hooks/use-verge'

export type ProxyMode = 'smart' | 'normal'

// 每个模式记住自己的 TUN 偏好；激活模式时应用它的偏好（切换模式自动切换 TUN）。
const TUN_MEMORY_KEY: Record<ProxyMode, string> = {
  smart: 'smartclash.mode.tun.smart',
  normal: 'smartclash.mode.tun.normal',
}

const readTunMemory = (mode: ProxyMode): boolean | null => {
  const raw = localStorage.getItem(TUN_MEMORY_KEY[mode])
  return raw === '1' ? true : raw === '0' ? false : null
}

const writeTunMemory = (mode: ProxyMode, enabled: boolean) => {
  localStorage.setItem(TUN_MEMORY_KEY[mode], enabled ? '1' : '0')
}

interface ModeControlsProps {
  mode: ProxyMode
}

export const ModeControls = ({ mode }: ModeControlsProps) => {
  const { data, patch } = useSmartClash()
  const { verge, patchVerge } = useVerge()

  const enabled = data?.settings.enabled ?? false
  const active = mode === 'smart' ? enabled : !enabled
  const liveTun = verge?.enable_tun_mode ?? false
  // 运行中的模式跟随实时 TUN 状态；未运行的模式显示它自己的记忆值。
  const tunOn = active ? liveTun : (readTunMemory(mode) ?? liveTun)

  const switchMode = useLockFn(async () => {
    if (!data) return
    const outgoing: ProxyMode = enabled ? 'smart' : 'normal'
    const incoming: ProxyMode = enabled ? 'normal' : 'smart'
    writeTunMemory(outgoing, liveTun)
    await patch({ ...data.settings, enabled: mode === 'smart' })
    const remembered = readTunMemory(incoming)
    if (remembered !== null && remembered !== liveTun) {
      await patchVerge({ enable_tun_mode: remembered })
    }
  })

  const toggleTun = useLockFn(async () => {
    const next = !tunOn
    writeTunMemory(mode, next)
    if (active && next !== liveTun) {
      await patchVerge({ enable_tun_mode: next })
    }
  })

  const title = mode === 'smart' ? '智能代理' : '普通代理'

  return (
    <Paper
      variant="outlined"
      sx={{ mb: 2, p: 1.5, borderRadius: 3, display: 'flex', gap: 1.5, flexWrap: 'wrap' }}
    >
      <Button
        size="large"
        color={active ? 'success' : 'primary'}
        variant={active ? 'contained' : 'outlined'}
        disabled={!data}
        startIcon={<PowerSettingsNewRounded />}
        onClick={() => void switchMode()}
        sx={{ flex: { xs: '1 1 100%', sm: '0 1 auto' } }}
      >
        {active ? `${title}运行中 · 点击切换` : `启动${title}`}
      </Button>
      <Tooltip title={active ? '切换当前 TUN 虚拟网卡状态' : '调整该模式启动时使用的 TUN 状态'}>
        <Button
          size="large"
          color={tunOn ? 'success' : 'primary'}
          variant={tunOn ? 'contained' : 'outlined'}
          startIcon={mode === 'smart' ? <AutoModeRounded /> : <LanRounded />}
          onClick={() => void toggleTun()}
          sx={{ flex: { xs: '1 1 100%', sm: '0 1 auto' } }}
        >
          TUN {tunOn ? '已开启' : '已关闭'}
        </Button>
      </Tooltip>
      <Stack sx={{ justifyContent: 'center', minWidth: 0 }}>
        <Typography variant="caption" color="text.secondary">
          两个模式互斥：启动一个会自动停掉另一个；TUN 按模式记忆，切换时自动应用。
        </Typography>
      </Stack>
    </Paper>
  )
}
