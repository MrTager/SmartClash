import { Tabs, Tab } from '@mui/material'
import { useLocation, useNavigate } from 'react-router'

export const DiagnosticTabs = () => {
  const { pathname } = useLocation()
  const navigate = useNavigate()
  return <Tabs value={pathname === '/logs' ? '/logs' : '/unlock'} onChange={(_, value: string) => navigate(value)}>
    <Tab label="网络与服务测试" value="/unlock" />
    <Tab label="事件与日志" value="/logs" />
  </Tabs>
}
