import SwiftUI
import UniformTypeIdentifiers

struct MobileContentView: View {
    @EnvironmentObject private var store: LibraryStore
    @EnvironmentObject private var tunnel: TunnelController
    @State private var showAdd = false
    @State private var showImport = false
    @State private var confirmExport = false
    @State private var showExport = false
    @State private var document = ClashDocument(data: Data())
    @State private var pendingDelete: SubscriptionRecord?
    var body: some View {
        TabView {
            home.tabItem { Label("连接", systemImage: "network") }
            subscriptions.tabItem { Label("订阅", systemImage: "square.stack.3d.up") }
            nodeList.tabItem { Label("节点", systemImage: "point.3.connected.trianglepath.dotted") }
            about.tabItem { Label("设置", systemImage: "gearshape") }
        }
        .tint(.indigo)
        .alert("操作提示", isPresented: Binding(get: { store.error != nil }, set: { if !$0 { store.error = nil } })) {
            Button("好", role: .cancel) { store.error = nil }
        } message: { Text(store.error ?? "") }
        .sheet(isPresented: $showAdd) { AddSubscriptionView() }
        .fileImporter(isPresented: $showImport, allowedContentTypes: [.json, UTType(filenameExtension: "yaml") ?? .plainText, .plainText, .data]) { result in
            switch result { case .success(let url): store.importFile(url); case .failure: store.error = "未能打开选中的文件。" }
        }
        .confirmationDialog("导出文件包含节点密码与密钥", isPresented: $confirmExport, titleVisibility: .visible) {
            Button("确认导出到所选位置") {
                do { document = ClashDocument(data: try store.exportData()); showExport = true }
                catch { store.error = error.localizedDescription }
            }
            Button("取消", role: .cancel) { }
        } message: { Text("这将生成未加密的 Clash JSON。不要发到公开聊天、网盘或代码仓库。") }
        .fileExporter(isPresented: $showExport, document: document, contentType: .json, defaultFilename: "SmartClash-private") { result in
            if case .failure = result { store.error = "文件未成功导出。" }
            document = ClashDocument(data: Data())
        }
        .confirmationDialog("删除订阅及它的本地节点缓存？", isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }), titleVisibility: .visible) {
            Button("删除", role: .destructive) { if let item = pendingDelete { store.remove(item.id) }; pendingDelete = nil }
            Button("取消", role: .cancel) { pendingDelete = nil }
        }
        .task { await tunnel.refresh() }
    }
    private var home: some View {
        NavigationStack {
            Form {
                Section {
                    Label("iOS 自用管理版 · 未接通 VPN 内核", systemImage: "info.circle")
                        .font(.subheadline).foregroundStyle(.secondary)
                    Toggle("VPN 连接", isOn: Binding(get: { tunnel.connected }, set: { wanted in
                        if !wanted { tunnel.stop() }
                        else { do { try tunnel.start() } catch { store.error = error.localizedDescription } }
                    }))
                    .disabled(!tunnel.connected && !tunnel.canStart)
                    .accessibilityHint("当前版本仅管理订阅，未提供可运行的代理内核")
                    Text(tunnel.message).font(.caption).foregroundStyle(.secondary)
                }
                Section("代理模式") {
                    Picker("代理模式", selection: Binding(get: { store.library.trafficMode }, set: store.mode)) {
                        ForEach(TrafficMode.allCases) { Text($0.title).tag($0) }
                    }.pickerStyle(.segmented).disabled(store.busy || !store.writable)
                    Text("仅保存和导出选择；不会改变 iPhone 当前网络。").font(.caption).foregroundStyle(.secondary)
                }
                Section("链路模式") {
                    Picker("链路模式", selection: Binding(get: { store.library.linkPreference }, set: store.preference)) {
                        ForEach(LinkPreference.allCases) { Text($0.title).tag($0) }
                    }.pickerStyle(.segmented).disabled(store.busy || !store.writable)
                    Text(store.library.linkPreference.detail).font(.caption).foregroundStyle(.secondary)
                }
                Section("当前资料") {
                    LabeledContent("订阅", value: String(store.library.subscriptions.count))
                    LabeledContent("节点", value: String(store.nodes.count))
                    LabeledContent("已选节点", value: store.selected?.name ?? "尚未选择")
                    if let notice = store.notice { Text(notice).font(.caption).foregroundStyle(.secondary) }
                    Button("导出已选节点配置") { confirmExport = true }
                        .disabled(store.busy || (store.selected == nil && store.library.trafficMode != .direct))
                }
                if !store.writable {
                    Section("资料读取未完成") {
                        Text("为防止覆盖原件，当前禁止保存。解锁设备后可重试。").foregroundStyle(.secondary)
                        Button("重试读取") { store.reload() }
                    }
                }
            }.navigationTitle("SmartClash")
        }
    }
    private var subscriptions: some View {
        NavigationStack {
            List {
                Section {
                    Button { showAdd = true } label: { Label("添加 HTTPS 订阅", systemImage: "plus") }
                    Button { showImport = true } label: { Label("导入 Clash YAML / JSON", systemImage: "square.and.arrow.down") }
                }.disabled(store.busy || !store.writable)
                Section("本地订阅") {
                    if store.library.subscriptions.isEmpty { Text("还没有订阅，可从文件开始导入。").foregroundStyle(.secondary) }
                    ForEach(store.library.subscriptions) { item in
                        VStack(alignment: .leading, spacing: 5) {
                            Text(item.source.name).font(.headline)
                            Text(item.source.isLocal ? "离线配置" : (URL(string: item.source.url)?.host ?? "HTTPS 订阅"))
                                .font(.caption).foregroundStyle(.secondary)
                            Text(item.updatedAt, format: .dateTime.year().month().day().hour().minute())
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                        .swipeActions {
                            Button("删除", role: .destructive) { pendingDelete = item }
                            if !item.source.isLocal { Button("更新") { Task { await store.refresh(item) } }.tint(.indigo) }
                        }.disabled(store.busy)
                    }
                }
                Section { Text("仅导入节点定义；订阅中的脚本、远端控制器和自定义规则不会执行。更新失败保留旧缓存。").font(.caption).foregroundStyle(.secondary) }
            }
            .navigationTitle("订阅")
            .overlay { if store.busy { ProgressView("正在处理订阅…").padding().background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12)) } }
        }
    }
    private var nodeList: some View {
        NavigationStack {
            List {
                if store.nodes.isEmpty { Text("导入订阅后，节点会显示在这里。").foregroundStyle(.secondary) }
                ForEach(store.nodes) { node in
                    Button { store.choose(node) } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 5) {
                                Text(node.name).foregroundStyle(.primary)
                                Text((node.payload["type"] as? String ?? "未知协议").uppercased() + " · 未进行出网验证")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if store.library.selectedNodeID == node.id { Image(systemName: "checkmark.circle.fill").foregroundStyle(.indigo) }
                        }
                    }.disabled(store.busy || !store.writable)
                }
            }.navigationTitle("节点")
        }
    }
    private var about: some View {
        NavigationStack {
            Form {
                Section("本版能力") {
                    Text("支持 HTTPS 订阅、本地 Clash 文件、节点去重、节点选择与配置导出。")
                    Text("不支持真实 VPN、代理测速、自动选路或后台切换；没有接管手机流量。").foregroundStyle(.secondary)
                }
                Section("真机 VPN 的后续条件") {
                    Text("需要付费 Apple Developer 账号的 Network Extension 能力，以及经过构建和实测的嵌入式内核。")
                    Text("免费签名请选择 SmartClash-Manager scheme；普通 App 的签名配置需按 Xcode 提示定期更新。")
                }
                Section("隐私") {
                    Text("资料使用设备本地 Keychain 密钥加密，不启用云同步。删除 App 后能否恢复资料不作保证，请妥善保存原始订阅。")
                    Text("请求只发到你主动添加的订阅地址。不记录节点密码，也不上传日志。显式导出文件包含凭据。")
                }
                Section {
                    Link("项目源码", destination: URL(string: "https://github.com/MrTager/SmartClash")!)
                    Link("苹果开发能力说明", destination: URL(string: "https://developer.apple.com/help/account/reference/supported-capabilities-ios")!)
                    Text("SmartClash iOS 0.1 · 管理预览版").font(.caption).foregroundStyle(.secondary)
                }
            }.navigationTitle("设置")
        }
    }
}

private struct AddSubscriptionView: View {
    @EnvironmentObject private var store: LibraryStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var address = ""
    var body: some View {
        NavigationStack {
            Form {
                TextField("订阅名称", text: $name).textInputAutocapitalization(.never)
                SecureField("HTTPS 订阅地址（含令牌）", text: $address)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                Text("地址可能包含访问令牌，不会显示在订阅列表或错误详情中。请仅添加你信任的 HTTPS 来源。")
                    .font(.caption).foregroundStyle(.secondary)
                Button("添加") { Task { if await store.add(name: name, address: address) { address = ""; dismiss() } } }
                    .disabled(store.busy || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || address.isEmpty)
                if store.busy { ProgressView() }
            }
            .navigationTitle("添加订阅")
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("关闭") { address = ""; dismiss() }.disabled(store.busy) } }
        }.interactiveDismissDisabled(store.busy)
    }
}
