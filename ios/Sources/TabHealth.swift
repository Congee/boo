import Foundation

enum ActiveTabHealth: Equatable {
    case opening
    case detached
    case expired
    case unreachable(tabId: UInt32)
    case exited(tabId: UInt32)
    case reachable(tabId: UInt32)

    var issue: String? {
        switch self {
        case .opening:
            return nil
        case .detached:
            return "Runtime view is detached; tap Reconnect to reattach"
        case .expired:
            return "Runtime view expired; tap Connect to request a new tab"
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
        case .opening:
            return "opening runtime view"
        case .detached:
            return "runtime view detached"
        case .expired:
            return "runtime view expired"
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
        case .reachable, .opening, .detached:
            return false
        case .expired, .unreachable, .exited:
            return true
        }
    }

    var allowsTransportSummary: Bool {
        !isDisconnected
    }
}

func resolveActiveTabHealth(
    activeTabId: UInt32?,
    tabs: [RemoteTabInfo],
    authenticated: Bool = true,
    runtimeViewId: UInt64? = nil,
    runtimeTabCount: Int? = nil,
    lastErrorKind: ClientWireErrorKind? = nil
) -> ActiveTabHealth {
    if let tabId = activeTabId {
        guard let tab = tabs.first(where: { $0.id == tabId }) else {
            return .unreachable(tabId: tabId)
        }
        if tab.childExited {
            return .exited(tabId: tabId)
        }
        return .reachable(tabId: tabId)
    }

    if lastErrorKind == .noActiveTab {
        return .expired
    }
    if authenticated, runtimeViewId != nil {
        if runtimeTabCount == 0 || tabs.isEmpty {
            return .expired
        }
        return .detached
    }
    return .opening
}
