import Foundation

private final class SecureRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        // 不把含令牌的订阅 URL 跟随到另一站点，也不降级 HTTP。
        guard let old = task.originalRequest?.url, let next = request.url,
              next.scheme?.lowercased() == "https", next.host?.lowercased() == old.host?.lowercased(),
              next.port == old.port, next.user == nil, next.password == nil else {
            completionHandler(nil); return
        }
        completionHandler(request)
    }
}

enum SubscriptionDownload {
    static func validateURL(_ text: String) throws -> URL {
        guard let url = URL(string: text.trimmingCharacters(in: .whitespacesAndNewlines)),
              url.scheme?.lowercased() == "https", let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.fragment == nil else {
            throw MobileFailure.message("订阅需为 HTTPS 地址，不允许用户名密码形式或 URL 片段。")
        }
        return url
    }
    static func fetch(_ text: String) async throws -> String {
        let url = try validateURL(text)
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20; config.timeoutIntervalForResource = 30
        config.urlCache = nil; config.requestCachePolicy = .reloadIgnoringLocalCacheData
        let session = URLSession(configuration: config, delegate: SecureRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        var request = URLRequest(url: url)
        request.setValue("application/yaml, application/json, text/plain", forHTTPHeaderField: "Accept")
        do {
            let (stream, response) = try await session.bytes(for: request)
            guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
                throw MobileFailure.message("订阅下载未成功；不接受跨站重定向，原缓存保持不变。")
            }
            if response.expectedContentLength > Int64(MobileConfiguration.maxSubscriptionBytes) {
                throw MobileFailure.message("订阅超过 10 MB。")
            }
            var data = Data()
            for try await byte in stream {
                if data.count % 8192 == 0 { try Task.checkCancellation() }
                guard data.count < MobileConfiguration.maxSubscriptionBytes else {
                    throw MobileFailure.message("订阅超过 10 MB。")
                }
                data.append(byte)
            }
            guard let content = String(data: data, encoding: .utf8) else {
                throw MobileFailure.message("订阅不是 UTF-8 文本。")
            }
            return content
        } catch is CancellationError { throw CancellationError() }
        catch let error as MobileFailure { throw error }
        catch { throw MobileFailure.message("订阅网络请求失败，已保留旧缓存；错误详情已隐藏以保护订阅令牌。") }
    }
}
