import { BasePage } from '@/components/base'
import { ModeControls } from '@/components/smartclash/mode-controls'
import { SmartClashPanel } from '@/components/smartclash/smartclash-panel'

const SmartProxyPage = () => {
  return (
    <BasePage
      title="智能代理"
      contentStyle={{ padding: 2 }}
    >
      <ModeControls mode="smart" />
      <SmartClashPanel />
    </BasePage>
  )
}

export default SmartProxyPage
