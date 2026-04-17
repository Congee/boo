import Foundation

enum ClientWireMessageType {
    case authOk
    case authFail
    case sessionList
    case attached
    case detached
    case sessionCreated
    case sessionExited
    case fullState
    case delta
    case errorMsg
}

enum ClientWireEffect: Equatable {
    case none
    case listSessions
    case attach(UInt32)
}

struct ClientWireState: Equatable {
    var authenticated = false
    var protocolVersion: UInt16?
    var transportCapabilities: UInt32 = 0
    var serverBuildId: String?
    var sessions: [DecodedWireSessionInfo] = []
    var screen: DecodedWireScreenState?
    var attachedSessionId: UInt32?
    var lastError: String?
}

enum ClientWireReducer {
    static func reduce(message: ClientWireMessageType, payload: Data, state: inout ClientWireState) -> ClientWireEffect {
        switch message {
        case .authOk:
            state.authenticated = true
            if payload.count >= 6 {
                state.protocolVersion = payload.withUnsafeBytes {
                    UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
                }
                state.transportCapabilities = payload.withUnsafeBytes {
                    UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt32.self))
                }
                if payload.count >= 8 {
                    let buildLength = payload.withUnsafeBytes {
                        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 6, as: UInt16.self)))
                    }
                    if payload.count >= 8 + buildLength {
                        state.serverBuildId = String(data: payload[8..<(8 + buildLength)], encoding: .utf8)
                    }
                }
            }
            state.lastError = nil
            return .listSessions
        case .authFail:
            state.lastError = "Authentication failed"
            return .none
        case .sessionList:
            state.sessions = WireCodec.decodeSessionList(payload)
            return .none
        case .attached:
            guard payload.count >= 4 else { return .none }
            state.attachedSessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
            return .none
        case .detached, .sessionExited:
            state.attachedSessionId = nil
            return .none
        case .sessionCreated:
            guard payload.count >= 4 else { return .none }
            let sessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
            return .attach(sessionId)
        case .fullState:
            state.screen = WireCodec.decodeFullState(payload)
            return .none
        case .delta:
            guard var screen = state.screen else { return .none }
            guard WireCodec.applyDelta(payload, to: &screen) else { return .none }
            state.screen = screen
            return .none
        case .errorMsg:
            state.lastError = String(data: payload, encoding: .utf8) ?? "Remote error"
            return .none
        }
    }
}
