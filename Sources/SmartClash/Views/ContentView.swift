import SwiftUI
import AppKit
import ProxyLogic
@MainActor
struct ContentView: View {
    @EnvironmentObject var store: AppStore
    @State private var section: String? = "概览"
    @State private var sourceName = ""
    @State private var sourceURL = ""
    @State private var query = ""
    private let sections = [("概览", "network"), ("订阅聚合", "square.stack.3d.up"), ("链式代理", "link"), ("节点", "point.3.connected.trianglepath.dotted"), ("事件日志", "list.bullet.rectangle")]
    var body: some View {
        NavigationSplitView {
            List(selection: $section) {
                ForEach(sections, id: \.0) { item in Label(item.0, systemImage: item.1).tag(item.0) }
            }
            .navigationTitle("SmartClash")
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 6) {
                    Label("Intel · macOS", systemImage: "desktopcomputer")
                    Text("本地处理 · 私有订阅").font(.caption).foregroundStyle(.secondary)
                }.padding()
            }
        } detail: {
            Group {
                switch section ?? "概览" {
                case "订阅聚合": subscriptions
                case "链式代理": ChainView().environmentObject(store)
                case "节点": nodeList
                case "事件日志": eventLog
                default: overview
                }
            }
            .padding(24)
            .navigationTitle(section ?? "概览")
            .toolbar {
                if store.busy { ProgressView().controlSize(.small) }
                Button { store.perform { try await store.refresh() } } label: { Label("更新订阅", systemImage: "arrow.clockwise") }
                    .disabled(store.busy || store.tun || store.preferences.sources.isEmpty)
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
                        Button(store.dynamicMode && store.running ? "停止并恢复网络" : "一键打通") {
                            store.perform {
                                if store.dynamicMode && store.running { try await store.stop() }
                                else { try await store.oneClickConnect() }
                            }
                        }.buttonStyle(.borderedProminent).disabled(store.busy)
                        Text("聚合节点，监测 OpenAI API，自动寻找可用出口。")
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image(systemName: store.running ? "network.badge.shield.half.filled" : "network")
                        .font(.system(size: 46)).foregroundStyle(store.running ? .green : .secondary)
                }
                if store.recovery {
                    GroupBox {
                        HStack {
                            Label("检测到未恢复的网络设置", systemImage: "exclamationmark.triangle")
                            Spacer()
                            Button("恢复网络") { store.perform { try await store.stop() } }.disabled(store.busy)
                        }.padding(8)
                    }
                }
                HStack(spacing: 16) {
                    metric("聚合节点", value: "\(store.nodes.count)", icon: "square.stack.3d.up")
                    metric("内核状态", value: store.running ? "运行中" : "已停止", icon: "power")
                    metric("当前出口", value: store.selected.isEmpty ? "尚未连接" : store.selected, icon: "location")
                }
                GroupBox("代理控制") {
                    VStack(spacing: 18) {
                        HStack {
                            Label("系统代理", systemImage: "globe")
                            Spacer()
                            Text(store.systemProxy ? "已开启" : "已关闭").foregroundStyle(.secondary)
                            Button(store.systemProxy ? "关闭" : "一键开启") {
                                store.perform { try await store.toggleSystem() }
                            }.disabled(store.busy || store.recovery)
                        }
                        Divider()
                        HStack {
                            VStack(alignment: .leading, spacing: 4) {
                                Label("TUN 虚拟网卡", systemImage: "externaldrive.connected.to.line.below")
                                Text("接管更多应用流量，开启与关闭需管理员授权。")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button(store.tun ? "关闭 TUN" : "一键开启 TUN") {
                                store.perform { try await store.toggleTun() }
                            }.disabled(store.busy || store.recovery)
                        }
                        Divider()
                        HStack {
                            Button(store.running ? "停止内核并恢复网络" : "仅启动内核") {
                                store.perform { if store.running { try await store.stop() } else { try await store.start() } }
                            }.disabled(store.busy || store.recovery)
                            Spacer()
                            Text("HTTP / SOCKS · 127.0.0.1:17890").font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                    }.padding(12)
                }
                GroupBox("智能切换") {
                    VStack(alignment: .leading, spacing: 15) {
                        Toggle("OpenAI 不可用时自动切换节点", isOn: $store.preferences.automatic)
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
                                .disabled(!store.running || store.busy)
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
    private var subscriptions: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("多个订阅，一个入口").font(.largeTitle.bold())
            Text("导入 Clash YAML / JSON 订阅，按节点配置去重。更新失败保留缓存。普通模式每小时更新，TUN 模式请停止后手动更新。")
                .foregroundStyle(.secondary)
            GroupBox("添加订阅") {
                HStack {
                    TextField("名称", text: $sourceName).frame(width: 130)
                    SecureField("HTTPS 订阅链接", text: $sourceURL)
                    Button("添加") {
                        do { try store.addSource(name: sourceName, url: sourceURL); sourceName = ""; sourceURL = "" }
                        catch { store.error = error.localizedDescription }
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
                            Text(URL(string: source.url)?.host ?? "HTTPS 订阅").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("删除", role: .destructive) { store.removeSource(source) }
                            .disabled(!store.canChangeSources)
                    }.padding(.vertical, 7)
                }
            }.overlay { if store.preferences.sources.isEmpty { Text("尚未添加订阅").foregroundStyle(.secondary) } }
            GroupBox("聚合输出") {
                VStack(alignment: .leading, spacing: 12) {
                    Text("本机订阅链接（含随机访问令牌）").fontWeight(.medium)
                    HStack {
                        Text(store.subscriptionReady ? "http://127.0.0.1:17892/••••/config.yaml" : "订阅服务不可用")
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
    private var nodeList: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("节点与连通性").font(.largeTitle.bold())
                Spacer(); TextField("搜索节点", text: $query).frame(width: 220)
            }
            Text("备用节点在故障切换时通过独立监听端口检测，不改变当前业务出口。")
                .foregroundStyle(.secondary)
            List(store.routeNodes.filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }) { node in
                HStack(spacing: 12) {
                    Image(systemName: node.name == store.selected ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(node.name == store.selected ? .green : .secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(node.name).lineLimit(1)
                        if let status = store.statuses[node.id] {
                            Text("\(status.result.detail) · \(status.result.milliseconds) ms · \(status.checked.formatted(date: .omitted, time: .standard))")
                                .font(.caption).foregroundStyle(.secondary)
                        } else { Text("尚未检测").font(.caption).foregroundStyle(.secondary) }
                    }
                    Spacer()
                    Button("选择") { store.perform { try await store.select(node) } }.disabled(store.busy || !store.running)
                }.padding(.vertical, 6)
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
