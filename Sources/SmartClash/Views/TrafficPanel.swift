import SwiftUI
import Charts

@MainActor
struct TrafficPanel: View {
    @ObservedObject var traffic: TrafficStore
    var body: some View {
        GroupBox("上传与下载流量") {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 12) {
                    totals("实时速度", upload: TrafficStore.bytes(traffic.uploadRate) + "/s", download: TrafficStore.bytes(traffic.downloadRate) + "/s")
                    totals("本次内核运行累计", upload: TrafficStore.bytes(traffic.sessionUpload), download: TrafficStore.bytes(traffic.sessionDownload))
                    totals("历史累计（本应用记录）", upload: TrafficStore.bytes(traffic.lifetimeUpload), download: TrafficStore.bytes(traffic.lifetimeDownload))
                }
                Chart(traffic.points) { point in
                    LineMark(x: .value("时间", point.time), y: .value("字节/秒", Double(point.upload)))
                        .foregroundStyle(by: .value("方向", "上传"))
                    LineMark(x: .value("时间", point.time), y: .value("字节/秒", Double(point.download)))
                        .foregroundStyle(by: .value("方向", "下载"))
                }
                .chartForegroundStyleScale(["上传": Color.cyan, "下载": Color.blue])
                .chartXAxis(.hidden).chartYAxisLabel("B/s").frame(height: 115)
                HStack {
                    Circle().fill(traffic.connected ? Color.green : Color.secondary).frame(width: 7, height: 7)
                    Text(traffic.connected ? "内核实时数据 · 最近 60 秒" : "内核未运行或统计暂不可用")
                    Spacer()
                }.font(.caption).foregroundStyle(.secondary)
                Text("以本应用内核上报为准，测速和后台探测流量可能计入；不是全电脑流量，也不是订阅服务商账单。历史数据定期保存在本机。")
                    .font(.caption).foregroundStyle(.secondary)
            }.padding(10)
        }
    }
    private func totals(_ title: String, upload: String, download: String) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Label(upload, systemImage: "arrow.up").monospacedDigit()
            Label(download, systemImage: "arrow.down").monospacedDigit()
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
