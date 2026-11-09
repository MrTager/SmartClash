import SwiftUI
import UniformTypeIdentifiers

@main
struct SmartClashMobileApp: App {
    @StateObject private var store = LibraryStore()
    @StateObject private var tunnel = TunnelController()
    var body: some Scene {
        WindowGroup { MobileContentView().environmentObject(store).environmentObject(tunnel) }
    }
}

struct ClashDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var data: Data
    init(data: Data) { self.data = data }
    init(configuration: ReadConfiguration) throws { data = configuration.file.regularFileContents ?? Data() }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper { FileWrapper(regularFileWithContents: data) }
}
