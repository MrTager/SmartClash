// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "SmartClash", platforms: [.macOS(.v13)],
    products: [.executable(name: "SmartClash", targets: ["SmartClash"])],
    dependencies: [.package(url: "https://github.com/jpsim/Yams.git", exact: "6.2.2")],
    targets: [
        .target(name: "ProxyLogic", dependencies: ["Yams"]),
        .executableTarget(name: "SmartClash", dependencies: ["ProxyLogic"]),
        .executableTarget(name: "SmartClashImport", dependencies: ["ProxyLogic", "Yams"]),
        .testTarget(name: "ProxyLogicTests", dependencies: ["ProxyLogic"]),
        .testTarget(name: "SmartClashTests", dependencies: ["SmartClash", "ProxyLogic"])
    ]
)
