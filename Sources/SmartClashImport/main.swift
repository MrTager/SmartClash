import Foundation
import CryptoKit
import Darwin
import ProxyLogic
import Yams

// Local-only migration: no subscription download or system network changes.
struct PreparedProfile {
    let source: Source
    let cache: Data
    let nodes: [ProxyNode]
}

func yamlDocument(_ url: URL) throws -> [String: Any] {
    do {
        let data = try Data(contentsOf: url)
        guard data.count <= 10_000_000,
              let document = try Yams.load(yaml: String(decoding: data, as: UTF8.self)) as? [String: Any] else {
            throw ProxyError.message("配置文件为空、过大或格式不受支持")
        }
        return document
    } catch {
        // YAML and filesystem diagnostics can contain credentials; never echo them.
        throw ProxyError.message("无法安全读取配置文件；未输出其内容")
    }
}

func stableID(_ uid: String) -> UUID {
    let hex = SHA256.hash(data: Data(("SmartClash.ClashVerge:" + uid).utf8))
        .prefix(16).map { String(format: "%02x", $0) }.joined()
    let chars = Array(hex)
    let text = [0..<8, 8..<12, 12..<16, 16..<20, 20..<32]
        .map { String(chars[$0]) }.joined(separator: "-")
    return UUID(uuidString: text)!
}

func privateWrite(_ data: Data, to url: URL) throws {
    try data.write(to: url, options: .atomic)
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
}

func run() throws {
    umask(0o077)
    let fm = FileManager.default
    let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    var sourceDirectory = support.appendingPathComponent("io.github.clash-verge-rev.clash-verge-rev")
    var destination = support.appendingPathComponent("SmartClash")
    var apply = false
    var arguments = Array(CommandLine.arguments.dropFirst())
    while !arguments.isEmpty {
        let option = arguments.removeFirst()
        switch option {
        case "--apply": apply = true
        case "--check": break
        case "--source-directory", "--destination":
            guard !arguments.isEmpty else { throw ProxyError.message("缺少目录参数") }
            let directory = URL(fileURLWithPath: arguments.removeFirst(), isDirectory: true)
            if option == "--source-directory" { sourceDirectory = directory } else { destination = directory }
        default: throw ProxyError.message("用法：SmartClashImport [--check|--apply] [--source-directory 路径] [--destination 路径]")
        }
    }
    let document = try yamlDocument(sourceDirectory.appendingPathComponent("profiles.yaml"))
    guard let items = document["items"] as? [[String: Any]] else {
        throw ProxyError.message("未找到 Clash Verge 配置列表")
    }
    let profiles = items.filter { ["remote", "local"].contains($0["type"] as? String ?? "") }
    guard !profiles.isEmpty else { throw ProxyError.message("未找到可导入的本地或远程配置") }
    var prepared: [PreparedProfile] = []
    for item in profiles {
        guard let uid = item["uid"] as? String, let name = item["name"] as? String,
              let filename = item["file"] as? String, !filename.isEmpty,
              filename == URL(fileURLWithPath: filename).lastPathComponent else {
            throw ProxyError.message("配置索引包含无效的文件引用")
        }
        var address = ""
        if item["type"] as? String == "remote" {
            guard let text = item["url"] as? String, let url = URL(string: text),
                  url.scheme == "https", url.host != nil, url.user == nil, url.password == nil else {
                throw ProxyError.message("远程配置不是安全的 HTTPS 订阅；本次导入尚未写入任何内容")
            }
            address = text
        }
        var source = Source(name: name, url: address)
        source.id = stableID(uid)
        let raw = try yamlDocument(sourceDirectory.appendingPathComponent("profiles").appendingPathComponent(filename))
        guard let rows = raw["proxies"] as? [[String: Any]] else {
            throw ProxyError.message("有配置不含内嵌 proxies 节点；本次导入尚未写入任何内容")
        }
        // Cache only node definitions, never third-party rules, controllers, scripts or secrets.
        let cache = try JSONSerialization.data(withJSONObject: ["proxies": rows], options: [.sortedKeys, .withoutEscapingSlashes])
        let nodes = try SubscriptionParser.parse(String(decoding: cache, as: UTF8.self), source: source)
        prepared.append(PreparedProfile(source: source, cache: cache, nodes: nodes))
        print("\(source.name): \(nodes.count) 个节点 · \(source.isLocal ? "本地快照" : "HTTPS 订阅")")
    }
    let settings = destination.appendingPathComponent("preferences.json")
    var preferences: [String: Any] = [:]
    if fm.fileExists(atPath: settings.path) {
        guard let existing = try JSONSerialization.jsonObject(with: Data(contentsOf: settings)) as? [String: Any] else {
            throw ProxyError.message("现有 SmartClash 设置无法解析；拒绝覆盖")
        }
        preferences = existing
    }
    let oldSources: [Source]
    if let existing = preferences["sources"] {
        oldSources = try JSONDecoder().decode([Source].self, from: JSONSerialization.data(withJSONObject: existing))
    } else { oldSources = [] }
    let importedIDs = Set(prepared.map { $0.source.id })
    let sources = oldSources.filter { !importedIDs.contains($0.id) } + prepared.map(\.source)
    var batches = prepared.map(\.nodes)
    for source in oldSources where !importedIDs.contains(source.id) {
        let cache = destination.appendingPathComponent("subscription-\(source.id).yaml")
        let data = try Data(contentsOf: cache)
        batches.append(try SubscriptionParser.parse(String(decoding: data, as: UTF8.self), source: source))
    }
    let merged = SubscriptionParser.merge(batches)
    preferences["sources"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(sources))
    if preferences["interval"] == nil { preferences["interval"] = 60 }
    if preferences["automatic"] == nil { preferences["automatic"] = true }
    if preferences["secret"] == nil { preferences["secret"] = UUID().uuidString + UUID().uuidString }
    if preferences["subscriptionToken"] == nil { preferences["subscriptionToken"] = UUID().uuidString }
    guard let secret = preferences["secret"] as? String else { throw ProxyError.message("本地控制器凭据格式无效") }
    let runtime = try Configuration.make(nodes: merged, secret: secret, tun: false)
    print("合计：\(prepared.count) 份配置，\(prepared.reduce(0) { $0 + $1.nodes.count }) 条缓存节点；去重后共 \(merged.count) 个节点")
    guard apply else { print("校验通过，尚未写入；使用 --apply 执行导入"); return }
    let running = Process()
    running.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
    running.arguments = ["-x", "SmartClash"]
    running.standardOutput = FileHandle.nullDevice
    running.standardError = FileHandle.nullDevice
    try running.run(); running.waitUntilExit()
    guard running.terminationStatus == 1 else { throw ProxyError.message("请先正常退出 SmartClash，再执行导入") }
    try fm.createDirectory(at: destination, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    try fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: destination.path)
    let outputs = prepared.map { (destination.appendingPathComponent("subscription-\($0.source.id).yaml"), $0.cache) }
        + [(destination.appendingPathComponent("config.json"), runtime),
           (settings, try JSONSerialization.data(withJSONObject: preferences, options: [.prettyPrinted, .sortedKeys]))]
    let existingOutputs = outputs.filter { fm.fileExists(atPath: $0.0.path) }
    if !existingOutputs.isEmpty {
        let backup = destination.appendingPathComponent("pre-import-" + UUID().uuidString)
        try fm.createDirectory(at: backup, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        for (file, _) in existingOutputs { try privateWrite(Data(contentsOf: file), to: backup.appendingPathComponent(file.lastPathComponent)) }
        print("已备份原 SmartClash 设置与将更新的缓存")
    }
    // Commit the preferences last so sources never refer to unwritten caches.
    for (file, data) in outputs { try privateWrite(data, to: file) }
    print("导入完成；原 Clash Verge 文件与系统网络设置均未修改")
}

do { try run() }
catch let error as ProxyError {
    fputs("导入失败：\(error.localizedDescription)\n", stderr)
    exit(1)
} catch {
    fputs("导入失败：本地文件操作或数据格式错误；详细内容已隐藏以保护凭据\n", stderr)
    exit(1)
}
