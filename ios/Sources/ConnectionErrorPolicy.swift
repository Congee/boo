import Foundation

enum ConnectionErrorPolicy {
    static func suppressAutomaticReconnect(for error: String?) -> Bool {
        guard let error else { return false }
        return error.localizedCaseInsensitiveContains("server identity changed")
    }
}
