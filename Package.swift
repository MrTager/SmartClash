// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "SmartClash", platforms: [.macOS(.v13)],
    products: [.executable(name: "SmartClash", targets: ["SmartClash"])],
    dependencies: [.package(url: "https://github.com/jpsim/Yams.git", exact: "6.2.2")],
    targets: [
        .target(name: "ProxyLogic", dependencies: ["Yams"]),
        .target(name: "CProcessIdentity"),
        .target(name: "LifecycleSupport", dependencies: ["CProcessIdentity"]),
        .target(name: "PrivilegedSupport", dependencies: ["ProxyLogic", "LifecycleSupport"]),
        .executableTarget(name: "SmartClashGuardian", dependencies: ["LifecycleSupport", "CProcessIdentity"]),
        .testTarget(name: "LifecycleSupportTests", dependencies: ["LifecycleSupport", "SmartClashGuardian"]),
        .executableTarget(name: "SmartClashHelper", dependencies: ["PrivilegedSupport", "ProxyLogic"]),
        .executableTarget(name: "SmartClash", dependencies: ["ProxyLogic", "PrivilegedSupport", "LifecycleSupport", "CProcessIdentity"], resources: [.process("Resources")]),
        .executableTarget(name: "SmartClashImport", dependencies: ["ProxyLogic", "Yams"]),
        .testTarget(name: "ProxyLogicTests", dependencies: ["ProxyLogic"]),
        .testTarget(name: "PrivilegedSupportTests", dependencies: ["PrivilegedSupport", "ProxyLogic"]),
        .testTarget(name: "SmartClashTests", dependencies: ["SmartClash", "ProxyLogic"])
    ]
)
