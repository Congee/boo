import Foundation

enum ActiveTabHealth: Equatable {
    case inactive
    case unreachable(tabId: UInt32)
    case exited(tabId: UInt32)
    case reachable(tabId: UInt32)

    var issue: String? {
        switch self {
        case .inactive:
            return "No active tab"
        case .unreachable(let tabId):
            return "Tab \(tabId) is unreachable"
        case .exited(let tabId):
            return "Tab \(tabId) has exited"
        case .reachable:
            return nil
        }
    }

    var statusSummary: String? {
        switch self {
        case .inactive:
            return nil
        case .unreachable(let tabId):
            return "tab \(tabId) unreachable"
        case .exited(let tabId):
            return "tab \(tabId) exited"
        case .reachable:
            return "tab reachable"
        }
    }

    var isDisconnected: Bool {
        switch self {
        case .reachable:
            return false
        case .inactive, .unreachable, .exited:
            return true
        }
    }

    var allowsTransportSummary: Bool {
        !isDisconnected
    }
}

func resolveActiveTabHealth(activeTabId: UInt32?, tabs: [RemoteTabInfo]) -> ActiveTabHealth {
    guard let tabId = activeTabId else { return .inactive }
    guard let tab = tabs.first(where: { $0.id == tabId }) else {
        return .unreachable(tabId: tabId)
    }
    if tab.childExited {
        return .exited(tabId: tabId)
    }
    return .reachable(tabId: tabId)
}
