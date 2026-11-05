import SwiftUI
import AppKit
import ProxyLogic
@MainActor
struct ContentView: View {
    @EnvironmentObject var store: AppStore
    @State private var section: String? = "概览"
    @State private var sourceName = ""
    @State private var sourceURL = ""
    @State private var proxyPortInput = ""
    private let sections = [("概览", "network"), ("订阅聚合", "square.stack.3d.up"), ("链式代理", "link"), ("节点", "point.3.connected.trianglepath.dotted"), ("实时流量", "chart.xyaxis.line"), ("事件日志", "list.bullet.rectangle")]
    var body: some View {
        NavigationSplitView {
            List(selection: $section) {
                ForEach(sections, id: \.0) { item in Label(item.0, systemImage: item.1).tag(item.0) }
            }
            .navigationTitle("SmartClash")
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 6) {
                    SidebarTrafficStatus(traffic: store.traffic)
                    Divider().padding(.vertical, 6)
                    Label("Intel · macOS", systemImage: "desktopcomputer")
                    Text("本地处理 · 私有订阅").font(.caption).foregroundStyle(.secondary)
                }.padding()
            }
        } detail: {
            Group {
                switch section ?? "概览" {
                case "订阅聚合": subscriptions
                case "链式代理": ChainView().environmentObject(store)
                case "节点": NodesView().environmentObject(store)
                case "实时流量": trafficPage
                case "事件日志": eventLog
                default: overview
                }
            }
            .padding(24)
            .navigationTitle(section ?? "概览")
            .toolbar {
                Button { store.perform { try await store.refresh() } } label: { Label("更新订阅", systemImage: "arrow.clockwise") }
                    .disabled(store.tun || store.preferences.sources.isEmpty)
            }
        }
        .frame(minWidth: 940, minHeight: 650)
        .alert("操作提示", isPresented: Binding(get: { store.error != nil }, set: { if !$0 { store.error = nil } })) {
            Button("好", role: .cancel) { store.error = nil }
        } message: { Text(store.error ?? "") }
    }
    private var overview: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                HStack {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("让连接持续在线").font(.largeTitle.bold())
                        Button(store.stopping ? "正在恢复网络…" : (store.running || store.busy ? "停止并恢复网络" : "一键打通")) {
                            if store.running || store.busy { store.requestStop() }
                            else { store.perform { try await store.oneClickConnect() } }
                        }.buttonStyle(.borderedProminent)
                        if store.busy && !store.stopping {
                            Text("正在后台处理，可随时停止或执行其他操作").font(.caption).foregroundStyle(.secondary)
                        }
                        Text("聚合节点，监测 OpenAI API，自动寻找可用出口。")
                            .foregroundStyle(.secondary)
                        Text(store.internetStatus).font(.caption).foregroundStyle(.secondary)
                        Text(store.openAIStatus).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if let icon = AppArtwork.icon {
                        Image(nsImage: icon).resizable().interpolation(.high).frame(width: 76, height: 76)
                            .accessibilityLabel("SmartClash 应用图标")
                    }
                }
                if store.recovery {
                    GroupBox {
                        HStack {
                            Label("检测到未恢复的网络设置", systemImage: "exclamationmark.triangle")
                            Spacer()
                            Button("恢复网络") { store.requestStop() }
                        }.padding(8)
                    }
                }
                TrafficRateBar(traffic: store.traffic)
                NetworkGlobePanel(activity: store.activity, nodes: store.visualizationNodes, trace: store.activeTrace)
                HStack(spacing: 16) {
                    metric("聚合节点", value: "\(store.nodes.count)", icon: "square.stack.3d.up")
                    metric("内核状态", value: store.running ? "运行中" : "已停止", icon: "power")
                    metric("当前出口", value: store.trafficMode == .direct ? "直连" : (store.selected.isEmpty ? "尚未连接" : store.selected), icon: "location")
                }
                TrafficModeControls().environmentObject(store)
                RoutingModeControls().environmentObject(store)
                TrafficPanel(traffic: store.traffic)
                GroupBox("代理控制") {
                    VStack(spacing: 18) {
                        HStack {
                            Label("系统代理", systemImage: "globe")
                            Spacer()
                            Text(store.systemProxy ? "已开启" : "已关闭").foregroundStyle(.secondary)
                            Button(store.systemProxy ? "关闭" : "一键开启") {
                                store.perform { try await store.toggleSystem() }
                            }.disabled(store.recovery)
                        }
                        Divider()
                        HStack {
                            VStack(alignment: .leading, spacing: 4) {
                                Label("TUN 虚拟网卡", systemImage: "externaldrive.connected.to.line.below")
                                Text("首次授权安装后台服务，之后开关无需重复授权。")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button(store.tun ? "关闭 TUN" : "一键开启 TUN") {
                                store.perform { try await store.toggleTun() }
                            }.disabled(store.recovery)
                        }
                        Divider()
                        HStack {
                            Label("本地代理端口", systemImage: "number")
                            TextField(String(Configuration.defaultProxyPort), text: $proxyPortInput)
                                .frame(width: 90).monospacedDigit()
                                .onAppear { proxyPortInput = String(Configuration.proxyPort) }
                                .onChange(of: store.preferences.proxyPort) { _ in
                                    proxyPortInput = String(Configuration.proxyPort)
                                }
                            Button("应用") {
                                let port = Int(proxyPortInput.trimmingCharacters(in: .whitespaces)) ?? -1
                                store.perform { try await store.changeProxyPort(port) }
                            }
                            .disabled(store.busy || store.stopping || store.recovery)
                            Spacer()
                            Text("范围 1024–65535；运行中修改会自动重启内核生效。")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Divider()
                        HStack {
                            Button(store.running || store.busy ? "停止内核并恢复网络" : "仅启动内核") {
                                if store.running || store.busy { store.requestStop() }
                                else { store.perform { try await store.start() } }
                            }.disabled(!store.running && !store.busy && store.recovery)
                            Spacer()
                            Text("HTTP / SOCKS · 127.0.0.1:\(Configuration.proxyPort)").font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                    }.padding(12)
                    HStack {
                        Text("系统代理与 TUN 共用一次授权；重开应用和重启电脑后继续有效。")
                            .font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Button("配置一次授权") {
                            store.perform {
                                try await PrivilegedClient.shared.prepare()
                                store.log("后台服务已就绪，系统代理与 TUN 开关无需重复授权")
                            }
                        }
                    }.padding(.horizontal, 12).padding(.bottom, 8)
                }
                GroupBox("智能切换") {
                    VStack(alignment: .leading, spacing: 15) {
                        Toggle("OpenAI 或网络不可用时自动切换节点", isOn: $store.preferences.automatic)
                            .onChange(of: store.preferences.automatic) { _ in store.savePreferences() }
                        HStack {
                            Text("检测间隔")
                            Picker("检测间隔", selection: $store.preferences.interval) {
                                Text("1 秒").tag(1); Text("5 秒").tag(5); Text("15 秒").tag(15)
                                Text("1 分钟").tag(60); Text("5 分钟").tag(300)
                            }.labelsHidden().frame(width: 160)
                                .onChange(of: store.preferences.interval) { _ in store.savePreferences() }
                            Spacer()
                            Button("立即检测 / 故障切换") { store.perform { try await store.inspectCurrent(force: true) } }
                                .disabled(!store.running)
                        }
                        Text("连续失败 2 次触发切换，切换冷却 30 秒。限流与不确定结果不触发切换。检测不重叠，实际周期包含请求耗时；推荐 1 分钟。")
                            .font(.caption).foregroundStyle(.secondary)
                        Text("未使用 API Key：检测仅确认 OpenAI API 的访问结果，不保证 ChatGPT 网页登录、验证码或账号可用性。切换影响新连接，已有请求可能需要重试。")
                            .font(.caption).foregroundStyle(.secondary)
                    }.padding(12)
                }
                if let recent = store.logs.first { Text(recent).font(.caption.monospaced()).foregroundStyle(.secondary) }
            }
        }
    }
    private func metric(_ title: String, value: String, icon: String) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Label(title, systemImage: icon).font(.caption).foregroundStyle(.secondary)
                Text(value).font(.title3.bold()).lineLimit(2).textSelection(.enabled)
            }.frame(maxWidth: .infinity, minHeight: 70, alignment: .leading).padding(8)
        }
    }
    private var trafficPage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text("实时流量").font(.largeTitle.bold())
                Text("每秒刷新当前代理的上传、下载速度和活跃连接。")
                    .foregroundStyle(.secondary)
                TrafficPanel(traffic: store.traffic, chartHeight: 220)
                NetworkGlobePanel(activity: store.activity, nodes: store.visualizationNodes, trace: store.activeTrace)
            }
        }
    }
    private var subscriptions: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("多个订阅，一个入口").font(.largeTitle.bold())
            Text("导入 Clash YAML / JSON 订阅，按节点配置去重。更新失败保留缓存，本地导入配置保留离线快照。普通模式每小时更新，TUN 模式请停止后手动更新。")
                .foregroundStyle(.secondary)
            GroupBox("添加订阅") {
                HStack {
                    TextField("名称", text: $sourceName).frame(width: 130)
                    SecureField("HTTPS 订阅链接", text: $sourceURL)
                    Button("添加") {
                        let name = sourceName, url = sourceURL
                        store.perform {
                            try store.addSource(name: name, url: url); sourceName = ""; sourceURL = ""
                        }
                    }.disabled(!store.canChangeSources || sourceURL.isEmpty)
                }.padding(8)
            }
            Text("添加或删除订阅前请停止内核。不支持仅含 proxy-providers 的配置、Base64 节点链接及节点链。")
                .font(.caption).foregroundStyle(.secondary)
            List {
                ForEach(store.preferences.sources) { source in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(source.name).fontWeight(.medium)
                            Text(source.isLocal ? "本地配置 · 离线快照" : (URL(string: source.url)?.host ?? "HTTPS 订阅")).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("删除", role: .destructive) { store.perform { store.removeSource(source) } }
                            .disabled(!store.canChangeSources)
                    }.padding(.vertical, 7)
                }
            }.overlay { if store.preferences.sources.isEmpty { Text("尚未添加订阅").foregroundStyle(.secondary) } }
            GroupBox("聚合输出") {
                VStack(alignment: .leading, spacing: 12) {
                    Text("本机订阅链接（含随机访问令牌）").fontWeight(.medium)
                    HStack {
                        Text(store.subscriptionReady ? "http://127.0.0.1:\(Configuration.aggregatePort)/••••/config.yaml" : "订阅服务不可用")
                            .font(.system(.caption, design: .monospaced))
                        Spacer()
                        Button("复制订阅链接") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(store.subscriptionURL, forType: .string) }
                            .disabled(store.nodes.isEmpty || !store.subscriptionReady)
                        Button("导出 YAML") { store.export() }.disabled(store.nodes.isEmpty)
                    }
                    Text("链接仅当前 Mac 可访问，客户端需保持运行。导出内容含节点密码，请妥善保管。其他客户端应选择“直连”更新本机订阅。")
                        .font(.caption).foregroundStyle(.secondary)
                }.padding(8)
            }
        }
    }
    private var eventLog: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("连接事件").font(.largeTitle.bold())
            Text("只显示连接与切换结果，不显示订阅 URL 或节点密码。内核诊断文件在本机保存。")
                .foregroundStyle(.secondary)
            List(store.logs, id: \.self) { Text($0).font(.system(.caption, design: .monospaced)).textSelection(.enabled) }
            Button("打开诊断目录") { NSWorkspace.shared.open(Files.root) }
        }
    }
}
