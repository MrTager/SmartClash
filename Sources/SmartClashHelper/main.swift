import Foundation
import PrivilegedSupport
import ProxyLogic
import Darwin

final class HelperService: NSObject, PrivilegedXPC, NSXPCListenerDelegate {
    private let installation: HelperInstallation
    private let queue = DispatchQueue(label: "com.smartclash.helper.operations")
    init(installation: HelperInstallation) { self.installation = installation }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        guard connection.effectiveUserIdentifier == installation.ownerUID else { return false }
        connection.setCodeSigningRequirement(installation.clientRequirement)
        connection.exportedInterface = NSXPCInterface(with: PrivilegedXPC.self)
        connection.exportedObject = self
        connection.resume()
        return true
    }
    func ping(withReply reply: @escaping (Int) -> Void) { reply(HelperLocation.version) }
    func setSystemProxy(_ snapshots: Data, enabled: Bool, withReply reply: @escaping (String?) -> Void) {
        execute(reply) {
            let items = try ProxySnapshot.decodeValidated(snapshots)
            for item in items {
                for args in item.arguments(enabling: enabled) { try self.run("/usr/sbin/networksetup", args) }
            }
        }
    }
    func startTun(_ configuration: Data, withReply reply: @escaping (String?) -> Void) {
        execute(reply) {
            try TunConfiguration.validate(configuration)
            guard !FileManager.default.fileExists(atPath: HelperLocation.corePlist) else {
                throw ProxyError.message("发现遗留 TUN 服务，请先恢复网络")
            }
            let root = HelperLocation.coreDirectory
            try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let core = root + "/mihomo"
            try self.run("/usr/bin/install", ["-o", "root", "-g", "wheel", "-m", "755", HelperLocation.directory + "/mihomo", core])
            let config = root + "/config.json"
            try self.write(configuration, to: config)
            try self.run(core, ["-t", "-d", root, "-f", config])
            let plist: [String: Any] = ["Label": HelperLocation.coreLabel,
                "ProgramArguments": [core, "-d", root, "-f", config], "RunAtLoad": true, "KeepAlive": false,
                "StandardOutPath": root + "/core.log", "StandardErrorPath": root + "/core.log",
                "WorkingDirectory": root, "Umask": 0o077]
            try self.write(PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0), to: HelperLocation.corePlist)
            // Leave the recovery record if bootstrap fails so the GUI can perform a fixed-label cleanup.
            try self.run("/bin/launchctl", ["bootstrap", "system", HelperLocation.corePlist])
        }
    }
    func stopTun(withReply reply: @escaping (String?) -> Void) {
        execute(reply) {
            let label = "system/" + HelperLocation.coreLabel
            if (try? self.run("/bin/launchctl", ["print", label])) != nil {
                try self.run("/bin/launchctl", ["bootout", label])
                // A cancelled or failed bootout leaves the TUN owning the default route.
                // Confirm the service is really gone instead of letting the GUI report
                // "network restored" while all traffic is still hijacked.
                var stopped = false
                for _ in 0..<30 {
                    if (try? self.run("/bin/launchctl", ["print", label])) == nil { stopped = true; break }
                    usleep(250_000)
                }
                guard stopped else { throw ProxyError.message("提权 TUN 内核服务仍在运行，可能仍接管系统流量") }
            }
            for path in [HelperLocation.corePlist, HelperLocation.coreDirectory] {
                if FileManager.default.fileExists(atPath: path) { try FileManager.default.removeItem(atPath: path) }
            }
        }
    }
    private func execute(_ reply: @escaping (String?) -> Void, _ operation: @escaping () throws -> Void) {
        queue.async {
            do { try operation(); reply(nil) }
            catch { reply(error.localizedDescription) }
        }
    }
    private func write(_ data: Data, to path: String) throws {
        try data.write(to: URL(fileURLWithPath: path), options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
    }
    private func run(_ executable: String, _ arguments: [String]) throws {
        let child = Process(); child.executableURL = URL(fileURLWithPath: executable); child.arguments = arguments
        child.environment = ["PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "LC_ALL": "C", "HOME": "/var/root"]
        // Do not return core diagnostics that could contain subscription credentials over XPC.
        child.standardOutput = FileHandle.nullDevice; child.standardError = FileHandle.nullDevice
        try child.run()
        let timeout = DispatchWorkItem { if child.isRunning { kill(child.processIdentifier, SIGKILL) } }
        DispatchQueue.global().asyncAfter(deadline: .now() + 20, execute: timeout)
        child.waitUntilExit(); timeout.cancel()
        guard child.terminationStatus == 0 else { throw ProxyError.message("后台网络操作失败（\(URL(fileURLWithPath: executable).lastPathComponent)，\(child.terminationStatus)）") }
    }
}

do {
    guard getuid() == 0 else { throw ProxyError.message("后台服务只能由系统 launchd 启动") }
    let installation = try PropertyListDecoder().decode(HelperInstallation.self, from: Data(contentsOf: URL(fileURLWithPath: HelperLocation.manifest)))
    guard installation.version == HelperLocation.version, installation.ownerUID > 0 else { throw ProxyError.message("后台服务安装记录无效") }
    try CodeIdentity.validate(installation.clientRequirement)
    let delegate = HelperService(installation: installation)
    let listener = NSXPCListener(machServiceName: HelperLocation.label)
    listener.setConnectionCodeSigningRequirement(installation.clientRequirement)
    listener.delegate = delegate; listener.resume()
    withExtendedLifetime(delegate) { RunLoop.current.run() }
} catch {
    fputs("SmartClashHelper: \(error.localizedDescription)\n", stderr)
    exit(1)
}
