import { Box, Grid } from '@mui/material'

import { BasePage } from '@/components/base'
import { CurrentProxyCard } from '@/components/home/current-proxy-card'
import { ModeControls } from '@/components/smartclash/mode-controls'

const NormalProxyPage = () => {
  return (
    <BasePage
      title="普通代理"
      contentStyle={{ padding: 2 }}
    >
      <ModeControls mode="normal" />
      <Grid container spacing={1.5} columns={{ xs: 6, sm: 6, md: 12 }}>
        <Grid size={6}>
          <CurrentProxyCard />
        </Grid>
        <Grid size={6}>
          <Box
            sx={{
              height: '100%',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              p: 3,
              borderRadius: 2,
              border: (theme) => `1px dashed ${theme.palette.divider}`,
            }}
          >
            <Box sx={{ textAlign: 'center' }}>
              <Box sx={{ fontWeight: 650, mb: 1 }}>手动选择节点</Box>
              <Box sx={{ color: 'text.secondary' }}>
                在左侧卡片中切换代理组并选择节点；普通代理模式下流量直接走订阅原有的规则与节点，不经过智能选路。
              </Box>
            </Box>
          </Box>
        </Grid>
      </Grid>
    </BasePage>
  )
}

export default NormalProxyPage
