import SwiftUI
import Charts

@MainActor
struct TrafficRateBar: View {
    @ObservedObject var traffic: TrafficStore
    var body: some View {
        HStack(spacing: 24) {
            rate("下载", value: traffic.downloadRate, symbol: "arrow.down", color: .blue)
            rate("上传", value: traffic.uploadRate, symbol: "arrow.up", color: .cyan)
            Spacer(minLength: 0)
            VStack(alignment: .trailing, spacing: 5) {
                Label(traffic.connected ? "流量实时更新" : (traffic.expectingData ? "正在读取…" : "尚未连接"),
                      systemImage: traffic.connected ? "waveform.path" : "pause.circle")
                    .font(.caption).foregroundStyle(.secondary)
                Text("本次 ↓ \(TrafficStore.bytes(traffic.sessionDownload))  ↑ \(TrafficStore.bytes(traffic.sessionUpload))")
                    .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
            }
        }
        .padding(14)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10))
    }
    private func rate(_ title: String, value: UInt64, symbol: String, color: Color) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).font(.title3.weight(.semibold)).foregroundStyle(color)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.caption).foregroundStyle(.secondary)
                Text(TrafficStore.bytes(value) + "/s").font(.system(.title3, design: .monospaced).bold()).monospacedDigit()
                    .contentTransition(.identity)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(title)速度 \(TrafficStore.bytes(value)) 每秒")
    }
}

@MainActor
struct SidebarTrafficStatus: View {
    @ObservedObject var traffic: TrafficStore
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 5) {
                Circle().fill(traffic.connected ? Color.green : .secondary).frame(width: 5, height: 5)
                Text("实时流量").font(.caption).foregroundStyle(.secondary)
            }
            Label(TrafficStore.bytes(traffic.downloadRate) + "/s", systemImage: "arrow.down")
                .foregroundStyle(.blue)
            Label(TrafficStore.bytes(traffic.uploadRate) + "/s", systemImage: "arrow.up")
                .foregroundStyle(.cyan)
        }.font(.system(.caption, design: .monospaced)).monospacedDigit()
    }
}

@MainActor
struct TrafficPanel: View {
    @ObservedObject var traffic: TrafficStore
    var chartHeight: CGFloat = 135
    var body: some View {
        GroupBox("上传与下载流量") {
            VStack(alignment: .leading, spacing: 14) {
                TrafficRateBar(traffic: traffic)
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Chart(traffic.points) { point in
                        LineMark(x: .value("时间", point.time), y: .value("字节/秒", Double(point.upload)))
                            .foregroundStyle(by: .value("方向", "上传"))
                        LineMark(x: .value("时间", point.time), y: .value("字节/秒", Double(point.download)))
                            .foregroundStyle(by: .value("方向", "下载"))
                    }
                    .chartForegroundStyleScale(["上传": Color.cyan, "下载": Color.blue])
                    .chartXScale(domain: context.date.addingTimeInterval(-60)...context.date)
                    .chartYScale(domain: 0...max(1024, Double(traffic.points.map { max($0.upload, $0.download) }.max() ?? 0) * 1.15))
                    .chartXAxis(.hidden)
                    .chartYAxis {
                        AxisMarks(position: .leading) { value in
                            AxisGridLine()
                            AxisValueLabel {
                                if let number = value.as(Double.self) {
                                    Text(TrafficStore.bytes(UInt64(max(0, number))) + "/s").font(.caption2)
                                }
                            }
                        }
                    }
                    .frame(height: chartHeight)
                    .overlay {
                        if traffic.points.isEmpty {
                            Text(traffic.expectingData ? "等待内核上报流量…" : "连接后显示最近 60 秒的真实流量曲线")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                HStack(spacing: 24) {
                    totals("本次内核运行累计", upload: traffic.sessionUpload, download: traffic.sessionDownload)
                    totals("历史累计（本应用记录）", upload: traffic.lifetimeUpload, download: traffic.lifetimeDownload)
                }
                HStack {
                    Circle().fill(traffic.connected ? Color.green : Color.secondary).frame(width: 7, height: 7)
                    Text(traffic.status)
                    Spacer()
                    Text("最近 60 秒")
                }.font(.caption).foregroundStyle(.secondary)
                Text("统计本应用内核的流量，含测速与后台探测；历史累计保存在本机。")
                    .font(.caption2).foregroundStyle(.secondary)
            }.padding(10)
        }
    }
    private func totals(_ title: String, upload: UInt64, download: UInt64) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text("↑ \(TrafficStore.bytes(upload))   ↓ \(TrafficStore.bytes(download))")
                .font(.system(.callout, design: .monospaced)).monospacedDigit()
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
