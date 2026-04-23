import Foundation

enum AttachedTabHealth: Equatable {
    case unattached
    case unreachable(tabId: UInt32)
    case exited(tabId: UInt32)
    case reachable(tabId: UInt32)

    var issue: String? {
        switch self {
        case .unattached:
            return "Tab is not attached"
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
        case .unattached:
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
        case .unattached, .unreachable, .exited:
            return true
        }
    }

    var allowsTransportSummary: Bool {
        !isDisconnected
    }
}

func resolveAttachedTabHealth(attachedTabId: UInt32?, tabs: [RemoteTabInfo]) -> AttachedTabHealth {
    guard let tabId = attachedTabId else { return .unattached }
    guard let tab = tabs.first(where: { $0.id == tabId }) else {
        return .unreachable(tabId: tabId)
    }
    if tab.childExited {
        return .exited(tabId: tabId)
    }
    return .reachable(tabId: tabId)
}
