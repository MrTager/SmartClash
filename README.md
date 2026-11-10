<p align="center">
  <img src="verge/src/assets/image/smartclash.png" alt="SmartClash" width="112" />
</p>

# SmartClash

**智能选路 · 订阅聚合 · 全球连接地球**

SmartClash 是基于 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev) **v2.5.10** 的桌面代理客户端，采用 **Tauri + React + Rust**，使用 **Mihomo** 内核。在上游的订阅、系统代理、TUN、规则与流量管理能力上，加入 SmartClash 的智能线路选择、备用检测、多跳链路和全球连接可视化。

当前仓库只维护 [`verge/`](verge/) 这一套桌面实现。上游基准为 [`c83f7ad`](https://github.com/clash-verge-rev/clash-verge-rev/commit/c83f7ad529c7c9cb3d2d54a0955ef8c1b22a411b)，版本记录见 [`UPSTREAM.json`](verge/UPSTREAM.json)。

> 当前为重构预览阶段。已在 Windows 完成构建、界面启动与自动化检查；真实节点故障切换、系统代理/TUN 接管和 macOS/Linux 打包仍需进一步验收。

## 核心功能

| 功能 | 当前实现 |
| --- | --- |
| 三种智能选路模式 | 低延迟、高速、AI，分别依据真实 HTTPS 延迟、下载吞吐和 OpenAI API 可达性选择候选 |
| 备用与故障切换 | 后台检测当前线路及备用候选，健康时保持出口，失败时按验证结果与冷却策略切换 |
| 多订阅聚合 | 当前订阅可叠加最多 64 个附加来源，按节点配置去重，同名不同配置保留并添加后缀 |
| 多跳链路 | 手动组合入口和出口，最多 128 个组合；可选自动探索最多 48 条双跳候选 |
| 国内直连规则 | 可选使用 [SmartClash-Rules](https://github.com/MrTager/SmartClash-Rules)，国内域名/IP 直连，其余流量走智能策略组 |
| 本机订阅与导出 | 带令牌的本机聚合订阅链接，以及 YAML 文件导出 |
| 全球连接地球 | 节点标记、请求连线、流动粒子、请求聚焦，支持拖动旋转与滚轮缩放 |
| 桌面代理管理 | 沿用上游系统代理、TUN、Mihomo 生命周期、规则/全局/直连模式、连接、日志和流量统计 |

### 选路模式

- **低延迟**：通过实际 HTTPS 请求测量延迟，在可用候选中排序。
- **高速**：显式重新选路时，每轮最多测试 6 个候选，各下载 1 MiB，以实际吞吐排序；后台监测只做连通性检查。
- **AI**：在 HTTPS 出网检查之外，访问 `https://api.openai.com/v1/models` 验证 OpenAI API 端点可达性，无需 API Key。结构化的缺少密钥响应可作为可达证据，429、验证挑战和未知响应不直接视为明确故障。

AI 检测不进行模型推理，也不代表 ChatGPT 网页登录、账号地区资格或模型权限可用。

## 开始使用

1. 在“订阅”页面导入配置并更新，沿用原有页面管理订阅来源。
2. 回到首页，打开“启用智能选路”，选择低延迟、高速或 AI 模式。需要聚合时，在“附加订阅来源”中选择其他配置。
3. 选择业务流量的使用方式：在“代理”页面将相关业务策略组选择为 **`SMARTCLASH`**；或启用“使用 SmartClash 国内直连规则”，让其替换当前订阅的分流规则。
4. 点击“按此模式重新选路”，等待真实检测结果，再按需开启系统代理或 TUN。

**仅启用智能选路开关，不会自动替换订阅中已有的业务选择。** 手动选择 `SMARTCLASH` 组中的节点会关闭自动选择；需要恢复时重新打开“自动维护备用与故障切换”。

### 地球动画

- 拖动或使用方向键旋转，滚轮缩放，双击或按 Home 复位。
- 青色标记为代理节点，橙色标记为请求目标；点击实时请求标签可聚焦对应连线。
- 节点位置基于服务器公网 IP，请求路径依据连接自己的链路及目标 IP，不会用当前选中节点替换全部历史连接。
- 缺少位置、私网或 fake-IP 请求计入“未定位”，不根据节点名称编造坐标。地理位置是估算，弧线不代表物理路由追踪。
- 可以关闭地球动画；系统“减少动态效果”偏好也会停用动态效果。

### 分流与导出

SmartClash 国内直连规则默认关闭。启用后使用自有规则仓库的 `direct.yaml`、`custom-direct.yaml` 和 `cncidr.yaml`，内核运行期间每小时更新；本机/私网及国内域名/IP 直连，未匹配流量走 `SMARTCLASH`。

“复制本机订阅链接”可供同一台电脑上的其他客户端读取；链接仅在 SmartClash 运行时有效，不是公网订阅服务。链接令牌及导出的 YAML 都应妥善保存，导出内容包含代理节点凭据。

## 构建与开发

### 环境

| 项目 | 版本或要求 |
| --- | --- |
| Node.js | 建议与上游环境一致，使用 26.13.0 |
| pnpm | 12.16.1，见 [`package.json`](verge/package.json) 的 `packageManager` |
| Rust | 1.99.1，见 [`rust-toolchain.toml`](verge/rust-toolchain.toml) |
| Windows | MSVC C++ Build Tools、Windows SDK、WebView2；正常构建目标为 `x86_64-pc-windows-msvc` |
| macOS/Linux | 对应平台的 [Tauri 构建依赖](https://tauri.app/start/prerequisites/)，目前尚未完成本项目的平台验收 |

### Windows 构建

```powershell
git clone https://github.com/MrTager/SmartClash.git
cd SmartClash/verge
pnpm install --frozen-lockfile
pnpm prebuild x86_64-pc-windows-msvc
pnpm build
```

`prebuild` 下载 Mihomo、匹配版本的系统服务和 Geo 资源，需要能访问 GitHub 发布资源。安装包位于 `verge/target/release/bundle/`；这些生成文件不随源码提交。

需要免安装预览目录时，在 `verge/` 中运行：

```powershell
.\build-preview.ps1
```

输出为仓库根目录的 `dist/SmartClash-Verge-Preview/SmartClash.exe`，包含配套内核、WebView 加载库及资源。该目录与前端输出 `verge/dist/` 分开，前端重新构建不会清空预览目录。预览是开发构建，尚未作为正式签名安装包验收。

### 开发运行

以下命令均在 `verge/` 中执行，运行桌面前先完成依赖安装和对应平台的 `prebuild`：

```powershell
pnpm dev:tauri  # 直接运行 Tauri 桌面应用，沿用当前系统服务状态
pnpm web:dev    # 仅运行前端，不启动 Rust 后端或代理内核
```

`pnpm dev` 使用上游服务开发流程，还需要在 `verge/` 同级目录准备匹配 **v2.7.7** 的 `clash-verge-service-ipc` 源码，即 `SmartClash/clash-verge-service-ipc/`；仅运行 `prebuild` 不满足这一开发模式的要求。服务开发细节见 [`CONTRIBUTING.md`](verge/CONTRIBUTING.md) 和 [桌面实现说明](verge/SMARTCLASH.md)。

### 检查

```powershell
pnpm typecheck
pnpm test
pnpm arch:check
pnpm web:build
cargo test -p clash-verge --lib --features clippy --locked
```

`clippy` 特性沿用上游约定，为代码检查跳过 Tauri 资源准备；通过库测试不等于安装包、系统服务或实网接管验收通过。

## 项目结构

```text
SmartClash/
├─ verge/
│  ├─ src/                    # React 界面、服务封装与状态
│  │  └─ components/smartclash/ # SmartClash 面板与地球渲染
│  ├─ src-tauri/              # Rust 后端、内核与系统集成
│  ├─ crates/                 # 上游公共组件
│  ├─ scripts/                # 资源准备与开发脚本
│  ├─ build-preview.ps1       # Windows 预览目录组装
│  ├─ SMARTCLASH.md           # 详细实现、构建与限制
│  └─ UPSTREAM.json           # 上游基准版本
├─ .github/workflows/         # 当前桌面主线检查
├─ README.md
└─ THIRD_PARTY_NOTICES.md
```

旧 Swift/macOS、iOS 与 Windows/egui 实现已从当前源码树移除，历史版本仍可通过 Git 提交记录查看。

## 验证状态与限制

截至 **2026-11-10**，本机验证结果如下：

| 项目 | 状态 |
| --- | --- |
| Rust 库测试 | 367 项通过 |
| 前端测试 | 37 项通过 |
| 类型检查、架构检查、前端生产构建 | 通过 |
| Windows 桌面预览 | 已构建、启动，验证地球旋转和缩放 |
| 真实节点故障切换、系统代理/TUN 接管 | 待实网验收 |
| macOS/Linux 打包与服务集成 | 待对应平台验收 |

- **系统服务**：当前主线使用 v2.7.7 服务协议。本机验收时，正在运行的旧 Clash Verge 服务与之不兼容，未替换现有服务。遇到提示可选择“稍后配置服务，先查看界面”；重新安装共享服务可能影响另一客户端。
- **自动选择**：当前线路健康时保留出口。普通明确故障需要连续两次检查，AI 明确故障可提前触发；自动切换有 30 秒冷却，失败候选退避 60 秒。切换不能保证已经建立的 TCP/WebSocket 请求不中断。
- **配置范围**：仅含 provider 节点的配置可手动使用 `SMARTCLASH`，自动隔离优化和本机聚合导出目前需要内联节点；附加订阅对来源策略组的引用需要改为显式节点。
- **定位**：公网 IP 通过 [ipwho.is](https://ipwho.is/) 查询，地球定位请求只发送 IP，不附带节点名称、订阅凭据或访问域名。成功结果在内存保留 24 小时，TUN fake-IP 目标可能无法定位。
- **数据与更新**：应用使用独立的 `io.github.mrtager.smartclash` 数据目录，开发模式带 `.dev` 后缀；旧版订阅需要手动导入。独立签名更新源准备完成前，SmartClash 应用自动更新保持关闭。

## 致谢与许可

感谢 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev)、[Mihomo](https://github.com/MetaCubeX/mihomo) 和 [Natural Earth](https://www.naturalearthdata.com/) 提供的基础能力与地图数据。国内直连规则来源于 [SmartClash-Rules](https://github.com/MrTager/SmartClash-Rules)。

项目遵循 GPL，保留上游许可证、来源及依赖记录。分发二进制时需同时履行对应源码和第三方声明义务，详见 [`LICENSE`](LICENSE)、[`verge/LICENSE`](verge/LICENSE) 与 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
