import Foundation
let args = CommandLine.arguments
let json = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: args[1]))) as! [String: Any]
let assets = json["assets"] as? [[String: Any]] ?? []
guard let item = assets.first(where: { $0["name"] as? String == args[2] }),
      let link = item["browser_download_url"] as? String,
      let digest = item["digest"] as? String,
      digest.hasPrefix("sha256:"), digest.count == 71,
      link.hasPrefix("https://github.com/MetaCubeX/mihomo/releases/download/") else {
    fputs("No matching official asset with SHA-256 digest. Check the pinned release.\n", stderr); exit(1)
}
let hash = String(digest.dropFirst(7))
guard hash.allSatisfy({ $0.isHexDigit }) else { exit(1) }
print(link); print(hash)
