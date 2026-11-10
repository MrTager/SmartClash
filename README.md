# SmartClash

SmartClash 直接基于 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev) **v2.5.10**，采用 Tauri + React + Rust。项目只保留这一套桌面主线，源码位于 [`verge/`](verge/)。上游版本和提交记录在 [`UPSTREAM.json`](verge/UPSTREAM.json)。

## 功能

- 沿用上游订阅管理、Mihomo 内核生命周期、系统代理、TUN、连接、规则、日志与流量统计。
- SmartClash 低延迟、高速、AI 三种选路模式，真实 HTTPS/OpenAI 检测、备用预热与故障切换。
- 多订阅聚合与去重、显式多跳、有限预算的自动双跳探索。
- 可选 SmartClash 国内直连规则、本机聚合订阅链接及 YAML 导出。
- 全球连接地球：旋转、缩放、请求弧线、流动粒子和请求聚焦；未知位置不会编造地理坐标。

## 开发与构建

需要 Node、pnpm、Rust 及各平台的 Tauri 构建工具。具体版本见 `verge/package.json` 和 `verge/rust-toolchain.toml`。Windows 正常构建使用 MSVC 及 C++ Build Tools。

```powershell
cd verge
pnpm install --frozen-lockfile
pnpm prebuild x86_64-pc-windows-msvc
pnpm dev
```

正式构建运行 `pnpm build`。`prebuild` 会准备相应平台的 Mihomo、服务与 Geo 资源。完整功能和构建说明见 [`verge/SMARTCLASH.md`](verge/SMARTCLASH.md)。

## 验证与当前边界

本机已完成前端构建、类型检查、37 项前端测试、367 项 Rust 库测试及架构检查；Windows 预览已启动并验证地球旋转和缩放。

本机正在使用的旧 Clash Verge 服务与当前主线要求的服务版本不兼容。尚未替换系统服务，真实节点故障切换与系统代理/TUN 接管仍需实网验收。启动时可选择“稍后配置服务，先查看界面”。macOS/Linux 打包和服务集成仍需对应平台验证。

应用数据保存在独立的 `io.github.mrtager.smartclash` 目录。已有订阅需要从“订阅”页面导入。SmartClash 应用自动更新在独立签名更新源准备好前保持关闭。

## 许可

遵循 GPL；保留上游许可证与来源记录，详见 [`LICENSE`](LICENSE)、[`verge/LICENSE`](verge/LICENSE) 和 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
