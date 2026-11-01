import Foundation
import ProxyLogic
struct Preferences: Codable {
    var sources: [Source] = []
    var interval: Int = 60
    var automatic: Bool = true
    var chain: ChainOptions? = nil
    var secret: String = UUID().uuidString + UUID().uuidString
    var subscriptionToken: String = UUID().uuidString
}
struct NodeStatus {
    var result: ProbeResult
    var checked: Date
}
