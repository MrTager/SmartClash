# SmartClash · Intel Mac 智能代理客户端

原生 SwiftUI 桌面客户端 + Mihomo 内核。目标：macOS 13 及以上、Intel x86_64（包括 2020 Intel MacBook Pro）。

**交付状态：源码实现版，尚未经过 macOS 编译和实机网络验收。当前生成环境为 Linux，没有 macOS SDK。此压缩包不是已验证的 DMG。** 首次使用请按照下面的步骤在 Mac 构建；编译通过、真实订阅、系统代理和 TUN 授权均需在 macOS 验证。

**本机更新记录（2026-11-04，v0.4.1）优先于历史交付状态：** 已在 Intel macOS 完成构建与自动化测试；新增连接前 HTTPS 出网验证、节点延迟测试与排序。TUN 管理员授权与接管后的实机验证是独立验收项，不能以编译通过替代。

## 快速构建

1. Mac 安装 Xcode 15+ 或 Swift 5.9+ Command Line Tools：`xcode-select --install`。Xcode 16.2 可以使用。
2. 解压项目，终端进入项目根目录。
3. 执行：

```bash
bash script/package.sh
```

也可以双击 `Build.command`。脚本会下载官方 Mihomo Intel 内核、校验 GitHub 发布资源的 SHA-256、解析 Yams 依赖、执行单元测试、编译 x86_64、签名并生成：

- `dist/SmartClash.app`
- `dist/SmartClash-Intel.dmg`

首次构建需要能访问 GitHub、api.github.com 和 GitHub 下载域名的网络。可先使用现有代理；终端不会总是自动使用系统代理，可设置：

```bash
export https_proxy=http://127.0.0.1:7890
export http_proxy=http://127.0.0.1:7890
bash script/package.sh
```

端口请改成现有代理的实际端口。内核固定为 Mihomo v1.19.33 compatible Intel 资产；如需手动提供可信内核：`MIHOMO_BINARY=/绝对路径/mihomo bash script/fetch_core.sh`。

默认采用本机 ad-hoc 签名，未进行 Developer ID 签名或 Apple 公证。自己在 Mac 上构建可本地使用；分发给其他人时，需要有效 Developer ID 和公证。可设置 `SIGNING_IDENTITY`，以及已配置的 Keychain `NOTARY_PROFILE` 后执行打包脚本。项目不包含开发者证书。

项目带 GitHub Actions Intel macOS 构建工作流，但未替你创建仓库、上传源码或运行远程构建。

## 使用

1. 启动应用，进入「订阅聚合」，添加一个或多个 **HTTPS Clash YAML / JSON 订阅**。
2. 点击「更新订阅」。节点按服务器、协议、认证和传输配置去重，来源名称不同不影响去重。重名节点会加稳定指纹后缀。
3. 点击「系统代理 → 一键开启」，允许系统授权。会开启所有当前启用网络服务的 HTTP、HTTPS、SOCKS 代理。
4. 需要虚拟网卡时点击「一键开启 TUN」，允许管理员授权。TUN 与系统代理可以单独控制。切换 TUN 会重启内核并恢复原系统代理设置；TUN 自行接管路由，无需同时开启系统代理。
5. 智能检测默认每分钟执行，支持 1 秒、5 秒、15 秒、1 分钟、5 分钟。网络请求串行、不会重叠；实际周期为请求/扫描耗时加设定间隔。改变间隔会在下一轮睡眠使用新值。
6. 复制「本机订阅链接」即可导入其他 Clash 客户端，或导出 YAML 文件。文件内容使用 YAML 1.2 兼容的 JSON 语法，Clash/Mihomo 可读取。链接只在此 Mac、此应用保持运行时可用；其他客户端应直连更新此链接。
7. 正常退出会先恢复旧系统代理，再停止内核 / 卸载 TUN 服务。若中途取消管理员授权，应用会保留运行并显示恢复提示。

系统代理和 TUN 之外还可「仅启动内核」，供指定应用使用本地 HTTP / SOCKS 端口。

## OpenAI 检测和切换

- 检测实际访问 `https://api.openai.com/v1/models`，不需要 API Key，不进行模型推理、不产生模型 API 调用费用。
- 返回结构化的缺少 API Key 的 401 响应，表示 API 端点可达；标准模型列表 JSON 也视为可达。
- 明确的地区限制、连接失败、超时、5xx 计为失败；连续 2 次失败，且距上次切换至少 30 秒，才开始选备用出口。
- 429 限流、403 验证挑战、未知页面不自动判为坏节点，避免把账号或平台限制误当出口失效。
- 备用节点通过独立 `PROBE` 组和 HTTP 监听端口检测，不影响正在使用的 `SMART` 组。
- 每轮最多扫描 12 个备用节点；失败节点冷却 60 秒；轮转覆盖后续节点。发现延迟小于 1500ms 的可用节点可提前结束，否则选择本批已验证节点中延迟最低者。
- 所有备用节点失败时保留当前节点并等待下一轮，不降级成直连外网。
- 只有故障时才换出口，不为几毫秒差异反复换 IP。手动选节点后同样冷却 30 秒。
- **这不保证 ChatGPT 网页登录、验证码、账号权限或地区资格可用，也不保证服务永不掉线。** 切换只作用于新连接；已经建立的 TCP、WebSocket 或正在生成的响应可能中断，需要应用自身重连/重试。
- 1 秒模式会更频繁访问 OpenAI；推荐使用默认 60 秒。

## 聚合格式与当前范围

支持含 `proxies` 的 Clash YAML / JSON，常见 SS、SSR、VMess、VLESS、Trojan、Hysteria/2、TUIC、HTTP、SOCKS5、Snell、AnyTLS 节点。具体字段是否被当前内核接受，会在启动前进行 `mihomo -t` 校验。

当前不支持 Base64 URI 列表（ss://、vmess:// 等）、仅有 proxy-providers 的远程配置、节点链引用、脚本/插件及依赖本地证书文件的节点。不会执行第三方订阅转换服务，也不会发送订阅给外部转换网站。若订阅服务能提供 Clash 格式，请选用其 Clash/Mihomo 订阅链接。

本地兼容修正：支持将旧式 VMess WebSocket 的 `ws-path` / `ws-headers` 规范化为 `ws-opts`；允许 Mihomo 内建的 SS `obfs` 传输（仅 `http` / `tls` 模式与 `host` 参数），仍拒绝外部插件和本地文件引用。配置输出不转义 `/`，以兼容内核的 YAML 解析器。

### 从本机 Clash Verge 导入全部配置

先正常退出 SmartClash，然后执行：

```bash
swift run --arch x86_64 SmartClashImport --check
swift run --arch x86_64 SmartClashImport --apply
bash script/package.sh
bash script/build_and_run.sh --install
```

导入器只读取用户级 Clash Verge `profiles.yaml` 以及对应的 `profiles/*.yaml`，不下载订阅、不访问其受保护系统目录、不改变系统代理或 TUN。来源数量由本机配置决定；所有 remote/local 配置均保留，各自缓存仅含 `proxies`。HTTPS 远程来源保留原更新地址，本地来源在更新时保留离线快照；若要再次导入原客户端中的本地修改，退出 SmartClash 后重新运行导入器。

默认读取 `~/Library/Application Support/io.github.clash-verge-rev.clash-verge-rev`，写入 `~/Library/Application Support/SmartClash`；可通过 `--source-directory` 和 `--destination` 指定其他目录。写入前先校验所有来源，已有且将被更新的 SmartClash 文件会备份至其数据目录中的 `pre-import-*` 私有目录。配置、缓存及备份含凭据，请勿提交到 Git。重复导入按原配置 UID 保留相同来源 ID，不会重复增加来源。

`--install` 仅在 `/Applications/SmartClash.app` 尚不存在时安装并启动，避免直接覆盖已有应用。普通开发仍使用 `bash script/build_and_run.sh --verify`。

正常模式手动/每小时更新订阅，更新失败保留该来源缓存。TUN 模式为避免普通 GUI 改写 root 配置，需先停止后手动更新。增删来源前需停止内核。首次添加后需手动更新。

本机聚合链接不是公网托管服务，不能给手机或其他电脑直接使用。导出订阅内含节点密码，使用者应按敏感凭据保存。

默认路由：回环地址与 RFC1918 私网直连，其余走所选节点。**没有内置国内外分流数据库**，避免首次启动下载额外 GeoIP/GeoSite 数据；可作为后续功能扩展。

## 权限、进程和恢复

- 普通模式：GUI 用户进程启动 Mihomo；TUN 模式：系统授权后将内核和配置复制为 root 所有的专用文件，通过固定 launchd 服务 `com.smartclash.core` 启动。
- TUN 不是 Apple Network Extension；使用 Mihomo 的 utun 和自动路由，需要管理员权限。GUI 本身不以 root 运行。
- root 配置在 `/Library/Application Support/SmartClashCore`，服务清单在 `/Library/LaunchDaemons/com.smartclash.core.plist`。停止 TUN 会卸载服务并删除这些专用文件。
- 系统代理开关先记录原设置，再修改；恢复成功后才删除备份。已有认证代理时拒绝修改，因为无法完整备份其密码。PAC 设置和代理绕过列表不由本应用修改；若启用了 PAC/自动发现策略，应用会提示先在系统设置关闭，避免优先级冲突。
- 应用工作数据在 `~/Library/Application Support/SmartClash`，目录 0700，订阅/配置/凭据文件 0600。日志仍可能包含服务器地址等诊断信息，请按敏感文件处理。
- 本地控制 API 仅监听回环地址，并使用随机密钥；聚合订阅使用独立随机 URL 令牌，仅监听回环地址。
- 普通内核记录 PID，异常退出后恢复操作只会终止可验证为本应用内核路径的遗留进程；不会按通用 mihomo 名称杀进程。
- 如强制退出、崩溃或重启留下设置，下次启动请点「恢复网络」。root TUN 服务在 GUI 崩溃后可能继续运行；正常退出或恢复网络会卸载。不要直接拖走正在使用的应用。
- 本地混合代理和 PROBE 端口不要求认证；同一 Mac 的其他本地进程可使用这些端口，设计适用于单用户可信设备。

若 GUI 无法启动，可在终端先卸载 TUN，再到系统设置 → 网络 → 当前服务 → 详细信息 → 代理，关闭指向 `127.0.0.1:17890` 的代理。仅在确认属于本应用时执行：

```bash
sudo launchctl bootout system/com.smartclash.core
sudo rm -f /Library/LaunchDaemons/com.smartclash.core.plist
sudo rm -rf '/Library/Application Support/SmartClashCore'
```

随后修复/重建应用再使用其保存的代理备份进行恢复。请勿在排查前删除应用工作数据中的 `proxy-backup.json`。

## 开发与验证

```bash
swift test --arch x86_64
bash script/build_and_run.sh
bash script/build_and_run.sh --verify
```

源码按 `ProxyLogic`（解析、去重、配置、健康判定）和 `SmartClash`（界面、进程、网络代理、订阅服务、监控）拆分。单元测试覆盖去重、不同凭据、远程配置隔离、PROBE 通道、敏感错误脱敏、导出可再次导入、响应分类、失败阈值与冷却等。测试源码已提供，需在 macOS 实际执行。

实机验收步骤与当前验证记录见 `VALIDATION.md`。构建脚本会在测试失败时停止，不会继续出包。

## 开源组件

本项目采用 GPL-3.0-or-later。代理内核 Mihomo 采用 GPL-3.0，Yams 采用 MIT；详见 `LICENSE` 和 `THIRD_PARTY_NOTICES.md`。此项目不是 Clash、Mihomo 或 OpenAI 的官方客户端。

## v0.2：智能双跳链式代理

新增「链式代理」页面。先停止内核，启用双跳，勾选入口候选（例如香港）和出口候选（例如美国），再开启系统代理或 TUN。

路径：本机 → 香港入口 → 美国出口 → OpenAI。美国出口节点通过 Mihomo `dialer-proxy` 指向原始香港入口。SMART 与 PROBE 都只选择生成的完整链路，后台按已有失败阈值、冷却和分批扫描策略在入口/出口组合之间切换。不会退回单跳，失败时不会直连公网。

不按节点名称自动猜测地区；需要你确认并选择候选池。当前路由仍按原默认规则：私网直连，其余流量都走所选链路，不仅限于 OpenAI。实际出口地区以服务商及检测结果为准。

最多 128 个入口×出口组合，排除同节点自连接；支持 TCP/HTTPS，暂不支持 UDP 专用协议及 SSR。订阅里的外部 dialer-proxy 仍不导入，链路仅由本机选择生成。旧设置文件可读取，默认关闭链式模式。聚合链接和 YAML 导出仍为原始节点池，不携带本机链式设置。

链路配置和新增 6 个测试尚未在 macOS 编译/执行；需按 VALIDATION.md 实机验证入口连接、出口 IP、整链 OpenAI 检测与故障切换。

官方实现说明：https://wiki.metacubex.one/en/config/proxies/dialer-proxy/

## v0.4：一键动态组网与预热备用（当前说明优先于上述历史版本）

界面无需选择入口、出口、地区或跳数。概览/链式代理页面只有一个核心按钮「一键打通」，成功后开启系统代理，再次点击停止并恢复网络；macOS 管理员授权仍由系统弹窗完成。原有系统代理和 TUN 控制继续保留。

后台按节点池、搜索轮次和资源预算生成无重复节点的可变长度路径，而非固定双跳/三跳；逐级 dialer-proxy 实现真实多跳。每轮最多 12 个候选，最大长度由节点数和该预算共同限制，不超过 12 跳。包括单跳候选，不承诺美国出口；地区约束需另加机制。UDP 专用协议仅参与单跳，TCP 兼容节点参与多跳。预算搜索不穷举全部排列，不保证全局最优或发现所有理论可用线路。

首次搜索按约 24 秒预算分批探测（正在进行的请求可额外耗时最多 8 秒），没有可用结果时保留后台探索，不显示成功、不启用新的系统代理。用户订阅需先添加；节点为空时按钮尝试更新现有订阅。

找到主线后，独立主线监控与后台备用探测同时运行：主线通过 SMART 业务监听检测，备用通过串行锁保护的 PROBE 监听检测，不改变业务出口。智能模式每次主线检测完成后等待 1 秒再检测；备用任务每次探测后等待 1 秒，不表示所有线路都每秒完成一次请求。

备用结果仅在最近 15 秒验证为 healthy 时有资格立即切换，按最近测得延迟优先。主线发生明确连接/地区失败后，有新鲜备用时跳过原 30 秒冷却和连续失败阈值，直接调用 SMART 组切换；限流/未知验证页仍不触发。无新鲜备用时按连续失败阈值重新探索。备用候选每约 60 秒动态刷新，保留主线、至多两条新鲜备用以及依赖定义，防止跨轮内存无限增长。配置更新短暂串行，主线检测会跳过忙碌时段。

这实现的是“预热备用 + 自动快速切换新连接”，不是现有 TCP 会话迁移。API 探测最坏 8 秒超时、调度、配置更新和本地控制调用都影响实际故障发现及切换耗时，不能保证零中断。正在生成的响应、已建立的 WebSocket/TCP 仍可能失败，调用端应重连/重试。未自动重放 POST，避免重复请求或扣费。最近验证可用也不保证切换瞬间仍然可用。

旧手动链式选项仅保留数据兼容，不再用于运行时选路。动态模式运行中暂停订阅刷新；停止后可更新再一键打通。聚合链接/YAML 仍仅导出原始节点池。代码尚未在 macOS 编译及真实线路验证。

## v0.4.1：出网验证、延迟测速与排序（本节优先于历史版本）

- 连接前先用本机非 TUN 内核检测 HTTPS 出网，按来源交错检查候选，优先复用新鲜的已测成功结果。只有真实业务监听也返回预期响应后，才开启新的系统代理。
- 默认单跳优先，不再直接把系统流量交给未验证的长链。多跳探索仍可在“链式代理”页面显式开启。
- TUN 前先验证线路，再保留同一节点启动 TUN；按当前默认路由解析物理出口，而不是假定 Wi-Fi 网卡。若发现另一 VPN 已接管默认路由，拒绝叠加 TUN。接管后重新检测，失败则尝试恢复；管理员若取消恢复授权，界面保留“恢复网络”提示。
- 为代理服务器域名提供独立的 IPv4 DNS，并在 TUN 模式绑定已识别的出口，降低 DNS 引导与 VPN 路由相互依赖的风险。
- “节点”页提供全部测速、单节点测速、进度、取消、延迟升序 / 名称 / 原始顺序。6 路并发，节点请求超时预算 5 秒；失败不会显示成 `0 ms`，未测与失败节点排在成功节点之后。
- 测速调用内核的独立节点延迟接口，不改变 SMART 业务组。若内核尚未运行，只启动本机内核，不开启系统代理或 TUN。结果私有保存在应用数据目录的 `latencies.json`。
- 选择节点也需通过出网测试，失败时保留或恢复原出口。普通 HTTPS 连通性与 OpenAI API 检查分开展示，避免把地区/账号限制当作所有网络均不可用。

安全更新已安装应用：先正常退出 SmartClash，再运行 `bash script/build_and_run.sh --update`；旧应用会保存在项目 `dist/SmartClash-installed-backup-*.app`。`ruby script/check_network.rb` 可使用临时隔离内核抽测缓存节点（`--all` 为全部），不会接管系统网络，也不输出节点凭据。

## v0.5.0：可视化链路、三种模式与流量面板

本节优先于历史版本说明。保留原订阅、本地缓存、图标与多跳探索设置。

1. **前后台隔离**：只有连接、更新、手动切换等前台操作显示忙碌状态。后台健康检测不再触发全页转圈，也不禁用停止按钮；停止可取消网络检测。重复健康信息去重发布，应用图标只解码一次，实时流量由独立观察对象更新。
2. **真实链路图**：依据生效节点及 `dialer-proxy` 引用，还原“本机 → 入口 → 中间节点 → 出口 → 目标”。展示跳数、协议、备用数和网络/OpenAI 状态；缺失引用或循环会明确标记不完整，不以动画冒充连接。
3. **低延迟模式**：优先选择实测 HTTPS 延迟较低且业务出网验证通过的候选。
4. **高速模式**：对最多 3 个候选分别通过独立 PROBE 监听下载 1 MiB，以下载字节/秒排序，展示 Mbps；一轮约消耗 3 MiB 测速流量。短时抽测不代表持续下载带宽，不会拿延迟假造吞吐，也不保证找到所有节点中的全局最优。
5. **AI 模式**：候选必须通过真实 OpenAI API 检查；结构化的无密钥认证响应可以确认 API 端点可达，不代表账号、网页登录、验证码或付费权限。其他模式不再仅因 OpenAI 地区限制而把正常互联网线路判为全断。
6. **实时流量**：从内核 `/traffic` 读取上传/下载速率及累计字节，显示最近 60 秒曲线、本次内核运行累计和历史累计。历史从本功能开始记录，不能还原旧版本未记录的数据。统计以内核上报为准，测速/探测可能计入；不是全电脑流量或服务商账单。

选择模式只保存偏好，连接时生效；已有连接需点击“按此模式重新选路”。验证失败会尝试保留原出口，不能把未确认的 AI/带宽结果标为成功。PROBE 组可测试原始节点与生成链路，不改 SMART 业务选择。

本机数据新增 `link-measurements.json` 和 `traffic-history.json`，仍使用私有目录/文件权限。构建 `dist` 不再退出正在使用的 `/Applications/SmartClash.app`；安装更新则要求先正常退出，以免擅自中断代理。

验收命令：

```bash
swift test --arch x86_64
# 仅在本机内核已运行时启用只读流量集成验证；离线 UI 验证不改网络。
SMARTCLASH_LIVE_TEST=1 SMARTCLASH_RENDER_PREVIEW=1 swift test --arch x86_64
bash script/package.sh
```

离线 UI 预览生成于 `dist/chain-ui-preview-0.5.0.png`，清楚标记为示例数据；最终安装后的真实连接与管理员授权验收仍需单独完成。
