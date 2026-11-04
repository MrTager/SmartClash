import Foundation

public struct TrafficSample: Codable, Equatable, Sendable {
    public let up: UInt64
    public let down: UInt64
    public let upTotal: UInt64?
    public let downTotal: UInt64?
    public init(up: UInt64, down: UInt64, upTotal: UInt64?, downTotal: UInt64?) {
        self.up = up; self.down = down; self.upTotal = upTotal; self.downTotal = downTotal
    }
}

public struct TrafficLedger: Codable, Equatable {
    public private(set) var lifetimeUpload: UInt64 = 0
    public private(set) var lifetimeDownload: UInt64 = 0
    public private(set) var lastUpload: UInt64 = 0
    public private(set) var lastDownload: UInt64 = 0
    public private(set) var runID: String?
    public init() {}
    public mutating func record(upload: UInt64, download: UInt64, runID newRunID: String) {
        let sameRun = runID == newRunID
        let uploadDelta = sameRun && upload >= lastUpload ? upload - lastUpload : upload
        let downloadDelta = sameRun && download >= lastDownload ? download - lastDownload : download
        lifetimeUpload += uploadDelta; lifetimeDownload += downloadDelta
        lastUpload = upload; lastDownload = download; runID = newRunID
    }
}
