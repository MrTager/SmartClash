import Foundation
import Darwin

public enum PublicIPAddress {
    public static func isPublic(_ text: String) -> Bool {
        var v4 = in_addr()
        if inet_pton(AF_INET, text, &v4) == 1 {
            let bytes = withUnsafeBytes(of: &v4) { Array($0) }
            let a = bytes[0], b = bytes[1], c = bytes[2]
            return !(a == 0 || a == 10 || a == 127 || a >= 224 ||
                (a == 100 && (64...127).contains(b)) || (a == 169 && b == 254) ||
                (a == 172 && (16...31).contains(b)) || (a == 192 && b == 168) ||
                (a == 192 && b == 0) || (a == 198 && (b == 18 || b == 19)) ||
                (a == 198 && b == 51 && c == 100) || (a == 203 && b == 0 && c == 113))
        }
        var v6 = in6_addr()
        guard inet_pton(AF_INET6, text, &v6) == 1 else { return false }
        let bytes = withUnsafeBytes(of: &v6) { Array($0) }
        if bytes.prefix(10).allSatisfy({ $0 == 0 }), bytes[10] == 255, bytes[11] == 255 {
            return isPublic(bytes.suffix(4).map(String.init).joined(separator: "."))
        }
        return bytes[0] & 0xe0 == 0x20 && !(bytes[0] == 0x20 && bytes[1] == 0x01 && bytes[2] == 0x0d && bytes[3] == 0xb8)
    }
}
