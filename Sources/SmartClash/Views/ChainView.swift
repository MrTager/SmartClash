import SwiftUI
@MainActor
struct ChainView: View {
    @EnvironmentObject var store: AppStore
    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Image(systemName: "network").font(.system(size: 64)).foregroundStyle(.tint)
            Text("自动寻找可用链路").font(.largeTitle.bold())
            Text(store.routingStatus).foregroundStyle(.secondary)
            Button(store.dynamicMode && store.running ? "停止并恢复网络" : "一键打通") {
                store.perform {
                    if store.dynamicMode && store.running { try await store.stop() }
                    else { try await store.oneClickConnect() }
                }
            }.buttonStyle(.borderedProminent).controlSize(.large).disabled(store.busy)
            if store.busy { ProgressView() }
            Text("自动组建链路并持续探测备用线路，故障时优先切换预热线路。无需选择地区或跳数。")
                .foregroundStyle(.secondary).multilineTextAlignment(.center)
            Spacer()
        }.frame(maxWidth: .infinity)
    }
}
