import NetworkExtension
import Foundation

/// 扩展边界已就位，但没有代理核心，必须在配置网络前拒绝启动。
/// 不能将桌面 CLI 作为 iOS 子进程，也不能把模拟连接当成真实 VPN。
final class PacketTunnelProvider: NEPacketTunnelProvider {
    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        completionHandler(NSError(domain: "SmartClash.PacketTunnel", code: 1001,
            userInfo: [NSLocalizedDescriptionKey: "此版本未链接可运行的 iOS 代理内核，VPN 未启动，未设置隧道路由或 DNS。"]));
    }
    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }
    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)?) {
        completionHandler?(try? JSONSerialization.data(withJSONObject: ["engineReady": false, "running": false]))
    }
}
