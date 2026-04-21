import Foundation

enum AttachedSessionHealth: Equatable {
    case unattached
    case unreachable(sessionId: UInt32)
    case exited(sessionId: UInt32)
    case reachable(sessionId: UInt32)

    var issue: String? {
        switch self {
        case .unattached:
            return "Session is not attached"
        case .unreachable(let sessionId):
            return "Session \(sessionId) is unreachable"
        case .exited(let sessionId):
            return "Session \(sessionId) has exited"
        case .reachable:
            return nil
        }
    }

    var statusSummary: String? {
        switch self {
        case .unattached:
            return nil
        case .unreachable(let sessionId):
            return "session \(sessionId) unreachable"
        case .exited(let sessionId):
            return "session \(sessionId) exited"
        case .reachable:
            return "session reachable"
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

func resolveAttachedSessionHealth(attachedSessionId: UInt32?, sessions: [SessionInfo]) -> AttachedSessionHealth {
    guard let sessionId = attachedSessionId else { return .unattached }
    guard let session = sessions.first(where: { $0.id == sessionId }) else {
        return .unreachable(sessionId: sessionId)
    }
    if session.childExited {
        return .exited(sessionId: sessionId)
    }
    return .reachable(sessionId: sessionId)
}
