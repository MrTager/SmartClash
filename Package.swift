// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "SmartClash", platforms: [.macOS(.v13)],
    products: [.executable(name: "SmartClash", targets: ["SmartClash"])],
    dependencies: [.package(url: "https://github.com/jpsim/Yams.git", exact: "6.2.2")],
    targets: [
        .target(name: "ProxyLogic", dependencies: ["Yams"]),
        .target(name: "PrivilegedSupport", dependencies: ["ProxyLogic"]),
        .executableTarget(name: "SmartClashHelper", dependencies: ["PrivilegedSupport", "ProxyLogic"]),
        .executableTarget(name: "SmartClash", dependencies: ["ProxyLogic", "PrivilegedSupport"], resources: [.process("Resources")]),
        .executableTarget(name: "SmartClashImport", dependencies: ["ProxyLogic", "Yams"]),
        .testTarget(name: "ProxyLogicTests", dependencies: ["ProxyLogic"]),
        .testTarget(name: "PrivilegedSupportTests", dependencies: ["PrivilegedSupport", "ProxyLogic"]),
        .testTarget(name: "SmartClashTests", dependencies: ["SmartClash", "ProxyLogic"])
    ]
)
