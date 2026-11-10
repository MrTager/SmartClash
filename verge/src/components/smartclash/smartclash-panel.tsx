import {
  Alert,
  Autocomplete,
  Box,
  Button,
  Chip,
  FormControlLabel,
  Paper,
  Stack,
  Switch,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from '@mui/material'
import { useState } from 'react'

import { useProfiles } from '@/hooks/use-profiles'
import { useSmartClash } from '@/hooks/use-smartclash'
import {
  getSmartClashMap,
  getSmartClashNodes,
  copySmartClashSubscription,
  saveSmartClashSubscription,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { useQuery } from '@/services/query-client'
import type {
  SmartClashMap,
  SmartClashMode,
  SmartClashSettings,
} from '@/services/smartclash-types'

import { NetworkGlobe } from './network-globe'

const emptyMap: SmartClashMap = { nodes: [], routes: [], unmapped: 0 }
const statuses: Record<string, string> = {
  disabled: '智能选路未启用',
  idle: '等待检测',
  probing: '后台检测中',
  ready: '线路已验证',
  manual: '保留手动选择',
  error: '检测未通过，保留当前线路',
  degraded: '当前线路待复检，保留连接',
  direct: '直连模式，暂停智能检测',
}

export function SmartClashPanel() {
  const { data, error, patch, optimize } = useSmartClash()
  const { profiles } = useProfiles()
  const sourceOptions = (profiles?.items ?? []).filter(
    (item) => item.uid && (item.type === 'remote' || item.type === 'local'),
  )
  const [focused, setFocused] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const { data: map } = useQuery({
    queryKey: ['smartclash-map'],
    queryFn: getSmartClashMap,
    refetchInterval: 5000,
  })
  const { data: nodes = [] } = useQuery({
    queryKey: ['smartclash-nodes'],
    queryFn: getSmartClashNodes,
    staleTime: 5000,
    refetchInterval: 5000,
  })
  const save = async (change: Partial<SmartClashSettings>) => {
    if (!data) return
    setPending(true)
    try {
      await patch({ ...data.settings, ...change })
    } catch {
      /* The mutation service owns error notices. */
    } finally {
      setPending(false)
    }
  }
  const run = async () => {
    try {
      await optimize()
    } catch {
      /* The mutation service owns error notices. */
    }
  }
  return (
    <Paper
      variant="outlined"
      sx={{
        mb: 2,
        p: { xs: 1.5, md: 2.5 },
        borderRadius: 3,
        overflow: 'hidden',
      }}
    >
      <Stack
        direction="row"
        sx={{
          alignItems: 'center',
          justifyContent: 'space-between',
          flexWrap: 'wrap',
          gap: 1,
          mb: 2,
        }}
      >
        <Box>
          <Typography variant="h5" sx={{ fontWeight: 750 }}>
            SmartClash
          </Typography>
          <Typography variant="body2" color="text.secondary">
            智能线路 · 全球连接
          </Typography>
        </Box>
        <Chip
          size="small"
          label={
            data ? (statuses[data.status] ?? data.status) : '正在读取线路状态'
          }
          color={data?.status === 'ready' ? 'success' : 'default'}
          variant="outlined"
        />
      </Stack>
      {(error || data?.error) && (
        <Alert severity="warning" sx={{ mb: 2 }}>
          {data?.error ?? String(error)}
        </Alert>
      )}
      <Box
        sx={{
          display: 'grid',
          gridTemplateColumns: {
            xs: '1fr',
            md: 'minmax(0, 1.4fr) minmax(245px, 1fr)',
          },
          gap: 2,
        }}
      >
        <Box sx={{ minWidth: 0 }}>
          <NetworkGlobe
            map={map ?? emptyMap}
            animated={data?.settings.globeAnimation ?? true}
            focused={focused}
          />
          <Stack direction="row" spacing={1} sx={{ mt: 1, flexWrap: 'wrap' }}>
            <Chip size="small" label={`已定位节点 ${map?.nodes.length ?? 0}`} />
            <Chip size="small" label={`请求连线 ${map?.routes.length ?? 0}`} />
            <Chip size="small" label={`未定位 ${map?.unmapped ?? 0}`} />
          </Stack>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: 'block', mt: 1 }}
          >
            青色为节点，橙色为请求目标。拖动旋转，滚轮缩放，双击复位。位置为公网
            IP 估算，连线不代表物理路由。
          </Typography>
        </Box>
        <Stack spacing={1.5} sx={{ minWidth: 0 }}>
          <FormControlLabel
            control={
              <Switch
                checked={data?.settings.enabled ?? false}
                disabled={!data || pending}
                onChange={(_, enabled) => {
                  void save({ enabled })
                }}
              />
            }
            label="启用智能选路"
          />
          <ToggleButtonGroup
            exclusive
            size="small"
            fullWidth
            value={data?.settings.mode ?? 'lowLatency'}
            disabled={!data || pending}
            onChange={(_, mode: SmartClashMode | null) => {
              if (mode) void save({ mode })
            }}
          >
            <ToggleButton value="lowLatency">低延迟</ToggleButton>
            <ToggleButton value="highSpeed">高速</ToggleButton>
            <ToggleButton value="ai">AI</ToggleButton>
          </ToggleButtonGroup>
          <Typography variant="body2" color="text.secondary">
            {data?.settings.mode === 'highSpeed'
              ? '以实际下载吞吐排序，每轮最多 6 个候选，各 1 MiB。'
              : data?.settings.mode === 'ai'
                ? '验证 HTTPS 出网及 OpenAI API 可达性；不代表网页登录或账号权限。'
                : '按实际 HTTPS 延迟选择线路，当前线路健康时保持连接。'}
          </Typography>
          <FormControlLabel
            control={
              <Switch
                size="small"
                checked={data?.settings.automatic ?? false}
                disabled={!data || pending}
                onChange={(_, automatic) => {
                  void save({ automatic })
                }}
              />
            }
            label="自动维护备用与故障切换"
          />
          <FormControlLabel
            control={
              <Switch
                size="small"
                checked={data?.settings.preferNonHongKong ?? true}
                disabled={!data || pending}
                onChange={(_, preferNonHongKong) => {
                  void save({ preferNonHongKong })
                }}
              />
            }
            label="优先非中国香港出口"
          />
          <FormControlLabel
            control={
              <Switch
                size="small"
                checked={data?.settings.globeAnimation ?? true}
                disabled={!data || pending}
                onChange={(_, globeAnimation) => {
                  void save({ globeAnimation })
                }}
              />
            }
            label="地球动画"
          />
          <Typography variant="caption" color="text.secondary">
            在「代理」中将业务策略组选择为
            SMARTCLASH，智能线路才会用于该组流量。原有系统代理、TUN、订阅和规则由
            Clash Verge Rev 管理。
          </Typography>
          <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
            当前智能链路：
            {data?.chain.length
              ? data.chain.join(' → ')
              : (data?.selected ?? '尚未选择')}
          </Typography>
          <Button
            variant="contained"
            disabled={!data?.settings.enabled || data.busy || pending}
            onClick={() => {
              void run()
            }}
          >
            {data?.busy ? '正在验证线路…' : '按此模式重新选路'}
          </Button>
        </Stack>
      </Box>
      {data && (
        <Stack spacing={1.5} sx={{ mt: 2 }}>
          <Typography sx={{ fontWeight: 650 }}>订阅聚合与分流</Typography>
          <Autocomplete
            multiple
            options={sourceOptions.flatMap((item) =>
              item.uid ? [item.uid] : [],
            )}
            value={data.settings.sourceProfiles ?? []}
            getOptionLabel={(uid) =>
              sourceOptions.find((item) => item.uid === uid)?.name ?? uid
            }
            disabled={pending}
            onChange={(_, sourceProfiles) => {
              void save({ sourceProfiles })
            }}
            renderInput={(params) => (
              <TextField
                {...params}
                label="附加订阅来源"
                helperText="与当前订阅聚合节点并去重；订阅更新沿用原有管理页面。"
                size="small"
              />
            )}
          />
          <FormControlLabel
            control={
              <Switch
                checked={data.settings.smartRules ?? false}
                disabled={pending}
                onChange={(_, smartRules) => {
                  void save({ smartRules })
                }}
              />
            }
            label="使用 SmartClash 国内直连规则（替换订阅分流规则）"
          />
          <Stack direction="row" spacing={1}>
            <Button
              size="small"
              disabled={!data.settings.enabled}
              onClick={() => {
                void mutate(copySmartClashSubscription, {
                  id: 'smartclash-export',
                }).catch(() => {})
              }}
            >
              复制本机订阅链接
            </Button>
            <Button
              size="small"
              disabled={!data.settings.enabled}
              onClick={() => {
                void mutate(saveSmartClashSubscription, {
                  id: 'smartclash-export',
                }).catch(() => {})
              }}
            >
              导出 YAML
            </Button>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            聚合链接仅在本机且应用运行时可用。链接与导出文件包含节点访问凭据，请妥善保存。
          </Typography>
          <Typography sx={{ fontWeight: 650 }}>显式多跳链路</Typography>
          <FormControlLabel
            control={
              <Switch
                checked={data.settings.exploreMultihop ?? false}
                disabled={pending || data.settings.chainEnabled}
                onChange={(_, exploreMultihop) => {
                  void save({ exploreMultihop })
                }}
              />
            }
            label="自动探索双跳候选（最多 48 条）"
          />
          <Box
            sx={{
              display: 'grid',
              gridTemplateColumns: { xs: '1fr', md: '1fr 1fr' },
              gap: 1.5,
            }}
          >
            <Autocomplete
              multiple
              options={nodes}
              value={data.settings.entryNodes}
              disabled={pending}
              onChange={(_, entryNodes) => {
                void save({ entryNodes })
              }}
              renderInput={(params) => (
                <TextField {...params} label="入口节点" size="small" />
              )}
            />
            <Autocomplete
              multiple
              options={nodes}
              value={data.settings.exitNodes}
              disabled={pending}
              onChange={(_, exitNodes) => {
                void save({ exitNodes })
              }}
              renderInput={(params) => (
                <TextField {...params} label="出口节点" size="small" />
              )}
            />
          </Box>
          <FormControlLabel
            control={
              <Switch
                checked={data.settings.chainEnabled}
                disabled={
                  pending ||
                  (!data.settings.chainEnabled &&
                    (!data.settings.entryNodes.length ||
                      !data.settings.exitNodes.length))
                }
                onChange={(_, chainEnabled) => {
                  void save({ chainEnabled })
                }}
              />
            }
            label="组合入口与出口（最多 128 个组合）"
          />
        </Stack>
      )}
      {!!map?.routes.length && (
        <Box sx={{ mt: 2 }}>
          <Typography sx={{ fontWeight: 650, mb: 1 }}>实时请求</Typography>
          <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 1 }}>
            {map.routes.slice(0, 12).map((route) => (
              <Chip
                key={route.id}
                size="small"
                variant={focused === route.id ? 'filled' : 'outlined'}
                onClick={() =>
                  setFocused(focused === route.id ? null : route.id)
                }
                label={route.host || route.locations.at(-1)?.ip || route.id}
                title={route.chain.join(' → ')}
              />
            ))}
          </Stack>
        </Box>
      )}
      {!!data?.measurements.length && (
        <Box sx={{ mt: 2, maxHeight: 180, overflow: 'auto' }}>
          {data.measurements.map((sample) => (
            <Stack
              key={sample.name}
              direction="row"
              sx={{ gap: 1, justifyContent: 'space-between', py: 0.5 }}
            >
              <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                {sample.name}
              </Typography>
              <Typography variant="caption" sx={{ whiteSpace: 'nowrap' }}>
                {sample.latencyMs === null
                  ? '未通过'
                  : `${sample.latencyMs} ms`}
                {sample.bytesPerSecond !== null
                  ? ` · ${((sample.bytesPerSecond * 8) / 1_000_000).toFixed(1)} Mbps`
                  : ''}
                {sample.aiReachable !== null
                  ? ` · AI ${sample.aiReachable ? '可达' : '受限'}`
                  : ''}
              </Typography>
            </Stack>
          ))}
        </Box>
      )}
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ display: 'block', mt: 2 }}
      >
        地理定位通过 ipwho.is 查询公网
        IP，不发送节点凭据或请求域名；结果在内存缓存 24 小时。仅有 provider
        的配置可手动使用 SMARTCLASH，自动优化目前需要内联节点。
      </Typography>
    </Paper>
  )
}
