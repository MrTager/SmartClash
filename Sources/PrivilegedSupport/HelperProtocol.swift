import Foundation
import Security
import ProxyLogic

public enum HelperLocation {
    public static let label = "com.smartclash.helper"
    public static let version = 2
    public static let executable = "/Library/PrivilegedHelperTools/com.smartclash.helper"
    public static let directory = "/Library/Application Support/SmartClashHelper"
    public static let manifest = directory + "/installation.plist"
    public static let plist = "/Library/LaunchDaemons/com.smartclash.helper.plist"
    public static let coreLabel = "com.smartclash.core"
    public static let corePlist = "/Library/LaunchDaemons/com.smartclash.core.plist"
    public static let coreDirectory = "/Library/Application Support/SmartClashCore"
}

@objc(SmartClashPrivilegedXPC)
public protocol PrivilegedXPC {
    func ping(withReply reply: @escaping (Int) -> Void)
    func setSystemProxy(_ snapshots: Data, enabled: Bool, withReply reply: @escaping (String?) -> Void)
    func startTun(_ configuration: Data, kernel: String, withReply reply: @escaping (String?) -> Void)
    func stopTun(withReply reply: @escaping (String?) -> Void)
}

public struct HelperInstallation: Codable, Equatable {
    public let version: Int
    public let build: String
    public let ownerUID: UInt32
    public let clientRequirement: String
    public let helperRequirement: String
    public let coreRequirement: String
    // 旧版安装记录没有该字段；nil 表示只有 Mihomo。
    public let kernelRequirements: [String: String]?
    public init(build: String, ownerUID: UInt32, clientRequirement: String, helperRequirement: String,
                coreRequirement: String, kernelRequirements: [String: String]? = nil) {
        version = HelperLocation.version; self.build = build; self.ownerUID = ownerUID
        self.clientRequirement = clientRequirement; self.helperRequirement = helperRequirement
        self.coreRequirement = coreRequirement; self.kernelRequirements = kernelRequirements
    }
}

public enum CodeIdentity {
    // codesign treats an unprefixed requirement argument as a filename.
    public static func commandLineRequirement(_ requirement: String) -> String { "=" + requirement }
    public static func requirement(at url: URL) throws -> String {
        var code: SecStaticCode?
        try check(SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(rawValue: 0), &code))
        guard let code else { throw ProxyError.message("无法读取应用签名") }
        try check(SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSStrictValidate), nil))
        var requirement: SecRequirement?
        try check(SecCodeCopyDesignatedRequirement(code, SecCSFlags(rawValue: 0), &requirement))
        guard let requirement else { throw ProxyError.message("应用缺少签名要求") }
        var text: CFString?
        try check(SecRequirementCopyString(requirement, SecCSFlags(rawValue: 0), &text))
        guard let text else { throw ProxyError.message("无法读取签名要求") }
        return text as String
    }
    public static func validate(_ text: String) throws {
        var requirement: SecRequirement?
        try check(SecRequirementCreateWithString(text as CFString, SecCSFlags(rawValue: 0), &requirement))
    }
    private static func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else { throw ProxyError.message("应用签名校验失败（\(status)），请重新构建或安装") }
    }
}

public struct ProxySnapshot: Codable, Equatable {
    public var service: String
    public var kind: String
    public var host: String
    public var port: Int
    public var enabled: Bool
    public init(service: String, kind: String, host: String, port: Int, enabled: Bool) {
        self.service = service; self.kind = kind; self.host = host; self.port = port; self.enabled = enabled
    }
    public static func decodeValidated(_ data: Data) throws -> [ProxySnapshot] {
        guard data.count <= 1_000_000 else { throw ProxyError.message("代理恢复记录过大") }
        let snapshots = try JSONDecoder().decode([ProxySnapshot].self, from: data)
        guard !snapshots.isEmpty, snapshots.count <= 1536 else { throw ProxyError.message("代理恢复记录数量无效") }
        var seen = Set<String>()
        for item in snapshots {
            guard ["webproxy", "securewebproxy", "socksfirewallproxy"].contains(item.kind),
                  !item.service.isEmpty, item.service.utf8.count <= 1024,
                  !item.service.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
                  item.host.utf8.count <= 1024,
                  !item.host.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
                  (0...65535).contains(item.port),
                  !item.enabled || (!item.host.isEmpty && item.port > 0),
                  seen.insert(item.service + "\0" + item.kind).inserted else {
                throw ProxyError.message("代理恢复记录包含无效网络服务、代理类型或地址")
            }
        }
        return snapshots
    }
    public func arguments(enabling: Bool) -> [[String]] {
        if enabling {
            return [["-set" + kind, service, "127.0.0.1", String(Configuration.proxyPort)],
                    ["-set" + kind + "state", service, "on"]]
        }
        var result: [[String]] = []
        if !host.isEmpty && port > 0 { result.append(["-set" + kind, service, host, String(port)]) }
        result.append(["-set" + kind + "state", service, enabled ? "on" : "off"])
        return result
    }
}
