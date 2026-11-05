import XCTest
import Foundation
import PrivilegedSupport
import ProxyLogic

final class PrivilegedSupportTests: XCTestCase {
    private func configuration(nodes: [ProxyNode] = [], ruleURL: String? = nil) throws -> Data {
        try Configuration.make(nodes: nodes, secret: "test-secret-with-enough-entropy", tun: true,
                               trafficMode: nodes.isEmpty ? .direct : .rule, ruleProviderBaseURL: ruleURL)
    }
    private func changed(_ data: Data, _ update: (inout [String: Any]) -> Void) throws -> Data {
        var object = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        update(&object)
        return try JSONSerialization.data(withJSONObject: object)
    }
    func testGeneratedTunConfigurationsAcceptedWithRulesAndTransportPaths() throws {
        let nodes = try SubscriptionParser.parse("""
        proxies:
          - {name: vmess, type: vmess, server: example.com, port: 443, uuid: test, network: ws, ws-opts: {path: /websocket}}
          - {name: shadowsocks, type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: test, plugin: obfs, plugin-opts: {mode: tls, host: example.com}}
        """, source: Source(name: "test", url: "https://example.com"))
        try TunConfiguration.validate(configuration())
        try TunConfiguration.validate(configuration(nodes: nodes, ruleURL: "http://127.0.0.1:\(Configuration.aggregatePort)/test-token/rules"))
    }
    func testTunRejectsArbitraryFilesAndUnexpectedExecutables() throws {
        let base = try configuration()
        for key in ["external-ui", "external-ui-url", "external-controller-unix", "script", "proxy-providers"] {
            XCTAssertThrowsError(try TunConfiguration.validate(changed(base) { $0[key] = "/tmp/untrusted" }), key)
        }
        for key in ["certificate", "private-key", "path", "plugin", "client-cert-path"] {
            XCTAssertThrowsError(try TunConfiguration.validate(changed(base) {
                $0["proxies"] = [["name": "unsafe", "type": "http", "server": "example.com", "port": 443, key: "/tmp/untrusted"]]
            }), key)
        }
    }
    func testTunRejectsPublicListenersAndWeakControllerSecrets() throws {
        let base = try configuration()
        for (key, value): (String, Any) in [("allow-lan", true), ("bind-address", "0.0.0.0"),
            ("external-controller", "0.0.0.0:19090"), ("secret", ""), ("mixed-port", 8080)] {
            XCTAssertThrowsError(try TunConfiguration.validate(changed(base) { $0[key] = value }), key)
        }
    }
    func testRuleProvidersCannotWriteOutsideFixedDirectoryOrFetchRemoteFiles() throws {
        let base = try configuration(ruleURL: "http://127.0.0.1:\(Configuration.aggregatePort)/token/rules")
        for (key, value) in [("path", "/etc/unsafe"), ("url", "file:///etc/passwd"), ("url", "https://example.com/rules")] {
            XCTAssertThrowsError(try TunConfiguration.validate(changed(base) {
                var providers = $0["rule-providers"] as! [String: [String: Any]]
                let name = providers.keys.sorted()[0]
                providers[name]![key] = value; $0["rule-providers"] = providers
            }))
        }
    }
    func testProxyCommandsKeepServiceNamesAndHostsAsLiteralArguments() throws {
        let service = "Wi-Fi ' ; $(touch /tmp/should-not-execute)"
        let snapshots = [ProxySnapshot(service: service, kind: "webproxy", host: "proxy.example.com", port: 8080, enabled: true)]
        let decoded = try ProxySnapshot.decodeValidated(JSONEncoder().encode(snapshots))
        XCTAssertEqual(decoded[0].arguments(enabling: true), [["-setwebproxy", service, "127.0.0.1", String(Configuration.proxyPort)], ["-setwebproxystate", service, "on"]])
        XCTAssertEqual(decoded[0].arguments(enabling: false), [["-setwebproxy", service, "proxy.example.com", "8080"], ["-setwebproxystate", service, "on"]])
    }
    func testLegacyEmptyDisabledProxyCanBeRestored() throws {
        let data = Data(#"[{"service":"Wi-Fi","kind":"socksfirewallproxy","host":"","port":0,"enabled":false}]"#.utf8)
        let item = try ProxySnapshot.decodeValidated(data)[0]
        XCTAssertEqual(item.arguments(enabling: false), [["-setsocksfirewallproxystate", "Wi-Fi", "off"]])
    }
    func testProxyValidationRejectsUnknownOperationsInvalidAddressesAndDuplicates() throws {
        let valid = ProxySnapshot(service: "Wi-Fi", kind: "webproxy", host: "", port: 0, enabled: false)
        var invalid = valid; invalid.kind = "dnsservers"
        var invalidPort = valid; invalidPort.port = 65536
        var invalidEnabled = valid; invalidEnabled.enabled = true
        var invalidService = valid; invalidService.service = "Wi-Fi\0unsafe"
        for items in [[invalid], [invalidPort], [invalidEnabled], [invalidService], [valid, valid], []] {
            XCTAssertThrowsError(try ProxySnapshot.decodeValidated(JSONEncoder().encode(items)))
        }
    }
    func testInstallationPinsOwnerAndCodeIdentitiesAcrossAppLaunches() throws {
        let installed = HelperInstallation(build: "11", ownerUID: 501, clientRequirement: "cdhash H\"0123456789012345678901234567890123456789\"",
            helperRequirement: "cdhash H\"0123456789012345678901234567890123456789\"", coreRequirement: "identifier mihomo")
        let saved = try PropertyListEncoder().encode(installed)
        XCTAssertEqual(try PropertyListDecoder().decode(HelperInstallation.self, from: saved), installed)
        try CodeIdentity.validate(installed.clientRequirement)
        XCTAssertNotEqual(installed, HelperInstallation(build: "11", ownerUID: 502, clientRequirement: installed.clientRequirement,
            helperRequirement: installed.helperRequirement, coreRequirement: installed.coreRequirement))
        XCTAssertThrowsError(try CodeIdentity.validate("not a valid requirement"))
    }
    func testActualCodeSigningRequirementWorksInInstallerCommandLine() throws {
        let requirement = try CodeIdentity.requirement(at: URL(fileURLWithPath: "/usr/bin/true"))
        func verify(_ executable: String) throws -> Int32 {
            let process = Process(); process.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
            process.arguments = ["--verify", "--strict", "-R", CodeIdentity.commandLineRequirement(requirement), executable]
            process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
            try process.run(); process.waitUntilExit(); return process.terminationStatus
        }
        XCTAssertEqual(try verify("/usr/bin/true"), 0)
        XCTAssertNotEqual(try verify("/usr/bin/false"), 0)
    }
}
