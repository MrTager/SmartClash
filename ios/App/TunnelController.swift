import Foundation
import Combine
import NetworkExtension

@MainActor
final class TunnelController: ObservableObject {
    @Published private(set) var status: NEVPNStatus = .invalid
    @Published private(set) var message = "此版本未集成代理内核，VPN 不可用。"
    private var manager: NETunnelProviderManager?
    private var observer: NSObjectProtocol?
    let hasTunnelTarget: Bool
    init() {
        #if SMARTCLASH_TUNNEL
        hasTunnelTarget = true
        #else
        hasTunnelTarget = false
        #endif
        observer = NotificationCenter.default.addObserver(forName: .NEVPNStatusDidChange, object: nil, queue: .main) { [weak self] notification in
            Task { @MainActor in
                guard let self, let connection = notification.object as? NEVPNConnection,
                      let managed = self.manager?.connection, connection === managed else { return }
                self.status = managed.status
            }
        }
    }
    deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }
    var connected: Bool { [.connected, .connecting, .reasserting].contains(status) }
    var canStart: Bool { false } // 未链接并实测可用内核前，绝不接管流量。
    func refresh() async {
        guard hasTunnelTarget else { return }
        do {
            let entries = try await NETunnelProviderManager.loadAllFromPreferences()
            let identifier = (Bundle.main.bundleIdentifier ?? "") + ".PacketTunnel"
            manager = entries.first { ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == identifier }
            status = manager?.connection.status ?? .invalid
        } catch { message = "无法读取本应用 VPN 状态；未修改系统 VPN 配置。" }
    }
    func start() throws {
        throw MobileFailure.message("尚未集成并验证 iOS 代理内核。VPN 真机运行还需付费开发者账号的 Network Extension 签名能力。")
    }
    func stop() {
        // 仅停止本 bundle 对应的配置，绝不停止其他 VPN App。
        manager?.connection.stopVPNTunnel()
    }
}
