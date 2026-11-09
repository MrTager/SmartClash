import Foundation
import Security
import CryptoKit

/// 订阅地址和节点凭据以 AES-GCM 加密文件存储；密钥在设备本地 Keychain 中。
struct SecureLibrary {
    let directory: URL
    private let service = "SmartClash.iOS.library-key.v1"
    init(directory: URL? = nil) {
        self.directory = directory ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("SmartClash", isDirectory: true)
    }
    private var file: URL { directory.appendingPathComponent("library.aesgcm") }
    private func key(create: Bool) throws -> SymmetricKey {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service, kSecAttrAccount as String: "encryption-key",
            kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
        var value: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &value)
        if status == errSecSuccess, let bytes = value as? Data, bytes.count == 32 { return SymmetricKey(data: bytes) }
        guard status == errSecItemNotFound, create else {
            throw MobileFailure.message("设备加密密钥暂不可用。请先解锁设备；现有资料不会被覆盖。")
        }
        let generated = SymmetricKey(size: .bits256)
        let bytes = generated.withUnsafeBytes { Data($0) }
        let insert: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service, kSecAttrAccount as String: "encryption-key",
            kSecValueData as String: bytes,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        let result = SecItemAdd(insert as CFDictionary, nil)
        if result == errSecDuplicateItem { return try key(create: false) }
        guard result == errSecSuccess else { throw MobileFailure.message("无法创建设备本地加密密钥。") }
        return generated
    }
    func load() throws -> MobileLibrary {
        guard FileManager.default.fileExists(atPath: file.path) else { return MobileLibrary() }
        let encrypted = try Data(contentsOf: file, options: .mappedIfSafe)
        guard encrypted.count <= MobileConfiguration.maxLibraryBytes + 2_000_000 else {
            throw MobileFailure.message("加密资料文件过大，已保留原件。")
        }
        do {
            let plain = try AES.GCM.open(AES.GCM.SealedBox(combined: encrypted), using: key(create: false))
            let library = try JSONDecoder().decode(MobileLibrary.self, from: plain)
            try MobileConfiguration.validate(library)
            return library
        } catch let error as MobileFailure { throw error }
        catch { throw MobileFailure.message("资料库无法解密或格式不兼容，已保留原件，不会自动重置。") }
    }
    func save(_ library: MobileLibrary) throws {
        try MobileConfiguration.validate(library)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.complete])
        var protectedDirectory = directory
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        try protectedDirectory.setResourceValues(values)
        let plain = try JSONEncoder().encode(library)
        guard let encrypted = try AES.GCM.seal(plain, using: key(create: true)).combined else {
            throw MobileFailure.message("资料加密失败。")
        }
        try encrypted.write(to: file, options: [.atomic, .completeFileProtection])
    }
}
