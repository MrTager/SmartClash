# SmartClash · Windows 版（Rust 重构）

这是 macOS SwiftUI 原版（`Sources/`）的 Windows 独立实现。**macOS 源码未做任何修改**，Windows 版放在 `windows/`，两者互不影响。

当前已同步原仓库 `620db3f`（v0.7.1）的源码，并在 Windows 版中重新实现 v0.6 引入的全球节点地球。macOS 专属的 SceneKit、特权后台服务（XPC/SMJobBless）无法直接移植，对应能力在 Windows 上以 UAC 提权方式实现。

## 技术选型

| 层 | macOS 原版 | Windows 版 |
| --- | --- | --- |
| 语言 | Swift 5.9 | Rust 1.99（GNU 工具链） |
| 界面 | SwiftUI / AppKit / Charts | egui + eframe（原生 Win32 窗口，无 WebView） |
| 内核 | Mihomo（macOS 资产） | Mihomo v1.19.33 `windows-amd64` |
| 系统代理 | `/usr/sbin/networksetup` | 当前用户注册表 `Internet Settings` + WinINET 刷新 |
| TUN | launchd 常驻服务 | 管理员权限（UAC）启动 Mihomo + `wintun.dll` |
| 订阅服务 | Network.framework | 标准库 TCP 监听 127.0.0.1:7892 |
| HTTP 请求 | URLSession / curl | 系统 `curl.exe`（与 macOS 行为对齐） |

## 目录结构

```
windows/
├─ Cargo.toml
├─ build.ps1                 # 编译 + 组装 dist + 打包 zip
├─ fetch-core.ps1            # 下载 / 校验 Mihomo Windows 内核（二进制不入库）
├─ tools/
│  └─ make_icon.py           # 从品牌图生成 app.ico 与窗口图标源码（纯标准库）
├─ resources/
│  ├─ app.ico                # 应用图标（16~256 多尺寸，含 PNG 大图）
│  ├─ mihomo.exe             # Mihomo 内核（Windows amd64）
│  ├─ wintun.dll             # TUN 虚拟网卡驱动
│  └─ world-land.json        # Natural Earth 简化陆地轮廓（全球节点视图）
└─ src/
   ├─ main.rs                # 入口 / 窗口 / 字体 / 动作分发
   ├─ app_icon.rs            # 由 tools/make_icon.py 生成的窗口图标位图
   ├─ logic/                 # 纯逻辑（从 ProxyLogic 移植）
   │  ├─ model.rs            # 数据模型、偏好设置
   │  ├─ subscription.rs     # Clash YAML 解析、去重指纹、安全校验
   │  ├─ configuration.rs    # Mihomo 配置生成与导出
   │  ├─ ranking.rs          # 延迟排序 / 候选轮转 / 模式排名
   │  ├─ chain.rs            # 链式代理组合与真实路径追踪
   │  ├─ dynamic_router.rs   # 多跳探索计划
   │  ├─ health.rs           # OpenAI / 出网探测分类
   │  └─ traffic.rs          # 流量账本
   ├─ services/              # 与 Windows 系统交互
   │  ├─ core_process.rs     # 启停内核、TUN 提权、遗留进程清理
   │  ├─ core_client.rs      # 本地控制 API、探测、测速、流量流
   │  ├─ system_proxy.rs     # 注册表备份 / 开启 / 恢复
   │  ├─ aggregate_server.rs # 本机订阅聚合 HTTP 服务
   │  ├─ subscription_service.rs
   │  └─ paths.rs / command.rs / network.rs
   ├─ state/                 # 应用状态与后台任务
   │  ├─ mod.rs              # Store、持久化、链路状态
   │  ├─ actions.rs          # 连接、选路、测速、故障切换、监控
   │  └─ traffic.rs          # 实时流量采集线程
   └─ ui/                    # 界面分区
      ├─ overview.rs / subscriptions.rs / chain.rs / nodes.rs / logs.rs
      ├─ globe.rs            # 全球节点分布地球（拖动旋转、地区聚合）
      └─ widgets.rs          # 卡片、指标、流量模式、线路模式、流量面板
```

## 构建

前置条件（无需管理员权限）：

1. Rust GNU 工具链：`rustup toolchain install stable-x86_64-pc-windows-gnu`
2. mingw-w64 链接器（例如 winlibs / w64devkit），并在 `%USERPROFILE%\.cargo\config.toml` 配置：

```toml
[target.x86_64-pc-windows-gnu]
linker = "C:/path/to/mingw64/bin/x86_64-w64-mingw32-gcc.exe"
ar = "C:/path/to/mingw64/bin/x86_64-w64-mingw32-gcc-ar.exe"
```

3. 内核二进制不入库（约 60 MB，与 macOS 版 `Resources/mihomo` 的约定一致）。首次构建先取内核，`wintun.dll` 已随仓库附带：

```powershell
cd windows
.\fetch-core.ps1          # 从 MetaCubeX/mihomo 发布页下载 v1.19.33 windows-amd64 并校验 SHA-256
# 已有内核时可指定本地文件：.\fetch-core.ps1 -Source "D:\path\mihomo.exe"
```

> 说明：Rust 自带的 self-contained MinGW 没有附带 `libwininet.a`，而 eframe → winapi 的默认特性会传 `-lwininet`。`build.rs` 会自动在 `windows/.linker/` 生成一个空的占位归档并加入链接搜索路径（本程序未调用 wininet 符号）。若已按上面配置了完整的 mingw-w64，这一层不影响链接。

然后：

```powershell
cd windows
.\build.ps1
```

产物：

- `windows/dist/SmartClash-Windows-0.8.1/` —— 免安装目录
- `windows/dist/SmartClash-Windows-0.8.1.zip` —— 绿色压缩包

直接运行目录中的 `SmartClash.exe` 即可。

## 图标与版本信息

exe 的图标、文件属性（`FileDescription` / `ProductName` / `FileVersion` / `Copyright`）由 `build.rs` 通过 `resources/app.ico` 写入，资源编译使用 mingw 的 `windres`（MSVC 工具链则是 `rc.exe`）。缺少资源编译器时只打印警告、不中断构建，只是 exe 没有图标。

图标沿用 macOS 版的品牌图形（蓝色圆角方块 + 白色 S 形线路与三个节点），由脚本从原图重采样生成，不手工维护位图：

```powershell
cd windows
python tools\make_icon.py     # 输出 resources/app.ico、resources/app-64.png、src/app_icon.rs
```

- 尺寸覆盖 16/20/24/32/40/48/64/96/128/256，128 与 256 用 PNG 压缩，其余是 32 位 DIB，兼顾体积与清晰度；
- 重采样在预乘 alpha 的线性光空间做面积平均，小尺寸不会发灰或偏暗；
- `src/app_icon.rs` 是 64×64 的 RGBA 常量，供 `ViewportBuilder::with_icon` 设置窗口/任务栏图标（`build.rs` 只负责 exe 静态图标，两者并用）。

## 与 macOS 版的差异（请务必知悉）

1. **系统代理**：只修改当前用户的 WinINET 代理（HTTP/HTTPS，`127.0.0.1:7890`）。Windows 没有与 macOS `networksetup` 完全等价的“逐网络服务”概念；SOCKS 由 Mihomo 的混合端口提供，但系统级 SOCKS 不单独写入注册表。开启前会拒绝已启用 PAC / 自动代理发现的系统配置，并先落盘 `proxy-backup.json`，停止时恢复。
2. **TUN**：需要管理员权限。点击后弹出 UAC，由提权 PowerShell 启动 Mihomo，`core.pid` / `tun.pid` 记录进程；关闭 TUN 或“恢复网络”时会再次请求授权并终止该 PID。若取消 UAC，应用会保留原状态并提示。**当前构建环境没有管理员权限，TUN 的真机接管效果尚未验收。**
3. **默认网卡**：Windows 版交给 Mihomo 自动探测（`auto-detect-interface: true`），不再像 macOS 版那样解析默认路由接口，也不会主动检测其他 VPN 的 TUN 冲突。若本机已有其他 TUN 类 VPN，请先关闭再开启 TUN，避免双重路由。
4. **Clash Verge 导入器**：macOS 版的 `SmartClashImport` 命令行工具未移植到 Windows。Windows 版可在「订阅聚合」中直接添加 HTTPS 订阅。
5. **界面**：使用 egui 重绘，布局与信息层级对齐原版，但不是像素级复刻。
6. **地球**：Windows 版用 egui 软件渲染的经纬投影球体，显示按节点名称推断的国家 / 城市分布，支持拖动旋转、双击复位；不是 SceneKit 的 3D 场景。连线是按当前出口（未连接时取节点最集中的地区）绘制的示意弧线，不代表实时请求路径。IP 归属由 `services/geoip.rs` 查询 `ipwho.is`：候选包括当前链路服务器、活跃请求的目标主机与其余节点，结果在本机缓存 24 小时。该查询会把公网 IP 发送给第三方；如不希望泄露节点服务器地址，可在防火墙或 DNS 层阻止 `ipwho.is`。
7. **规则订阅**：内核配置通过 mihomo `rule-providers` 订阅 [MrTager/SmartClash-Rules](https://github.com/MrTager/SmartClash-Rules) 的 `rules/direct.yaml`、`rules/custom-direct.yaml`、`rules/cncidr.yaml`，经 SMART 组下载并缓存到内核目录 `rules/`，内核运行期间每小时自动刷新；概览页「流量模式」中提供「立即更新规则」，通过内核 `/providers/rules/{name}` 逐个触发，失败保留旧规则。规则代理模式下本机 / 局域网、国内域名与国内 IP 直连，其余走 SMART；不再使用订阅内的第三方规则。**规则 / 全局 / 直连三种流量模式已实现**：切换通过 `/configs` PATCH 下发并回读确认，失败自动回滚；切换后断开旧连接；直连模式不选节点、允许在没有任何订阅时启动内核。v0.7.1 的签名校验特权后台服务属于 macOS 实现，Windows 使用 UAC + 独立进程。
8. **一键连接**：45 秒预算内先按入口服务器去重拨测（每个服务器只取一个节点），再扩展到订阅顺序的其余节点，避免同一台失效服务器耗尽时间。AI 模式优先使用已确认可达 OpenAI 的节点；若暂无确认结果，会先接通普通线路，再由后台每 30 秒复检并在状态栏显示 OpenAI 结果，不再因 OpenAI 单点验证失败而整体拒绝连接。AI 模式运行中若 OpenAI 探测明确失败（地区限制 / 离线），会跳过连续失败阈值，立即用独立 `PROBE` 组复检备用线路并切到最先确认可达的节点；30 秒内最多切换一次，避免线路抖动。

## 验证情况

系统代理与 TUN 可同时开启。切换 TUN 会重启内核，启动成功后恢复切换前的系统代理开关；单独关闭系统代理会保留 TUN 及其智能选路。停止内核或恢复网络会一并关闭两者。

当前构建已完成以下冒烟测试：

- `SmartClash.exe` 正常启动、渲染中文界面、全球节点地球与地区统计正常显示、关闭时返回码 0；
- 一键连接在真实订阅上约 15 秒接通，代理端口返回 `HTTP 204`，状态栏显示「HTTPS 出网正常」；
- 系统代理写入 `127.0.0.1:7890`，点击「停止并恢复网络」后 `ProxyEnable` 恢复为 0；
- 窗口使用普通层级，不会置顶遮挡其他应用；
- 本机聚合订阅端口 `127.0.0.1:7892` 可监听，正确令牌返回 200，错误令牌返回 404；
- 单元测试用真实 `mihomo.exe -t` 校验了由 Rust 生成的配置（`cargo test --release`），并覆盖规则 / 全局 / 直连三种模式的配置（含零订阅节点的直连配置）；
- 应用启动后连续运行 6 秒无崩溃退出，模式卡片与「当前出口」随模式联动。

TUN 接管仍需管理员权限，当前构建环境未做真机验收。首次使用请先准备测试订阅，并避免在重要网络环境中直接开启 TUN。

## 功能对照

已实现：订阅聚合与去重、节点延迟测试（6 并发）、OpenAI API 可达性检测、国内直连规则订阅（[MrTager/SmartClash-Rules](https://github.com/MrTager/SmartClash-Rules)）、规则代理 / 全局代理 / 直连三种流量模式、低延迟 / 高速 / AI 三种选路模式、全球节点分布地球、系统代理一键开关与恢复、TUN（需 UAC）、本地聚合订阅链接与 YAML 导出、实时流量面板与历史累计、后台故障切换、多跳链路探索开关、真实链路追踪、事件日志。

## 安全说明

- 配置、订阅缓存、代理备份、流量历史保存在 `%APPDATA%\SmartClash`，仅当前用户可访问；其中可能含节点密码，请按敏感凭据处理。
- 本地控制 API（9090）与聚合订阅（7892）仅监听 `127.0.0.1`，并使用随机密钥 / 令牌。
- 应用不会上传订阅到第三方转换服务，也不会执行订阅中的脚本或外部插件。
- 导出的 YAML 含节点密码，分享前请自行处理。

## 许可

跟随原仓库 GPL-3.0。Mihomo 与 wintun 分别遵循其上游许可证，详见原仓库 `THIRD_PARTY_NOTICES.md`。
