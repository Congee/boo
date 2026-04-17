import Foundation

@main
struct ProtocolCodecSelfTestMain {
    static func main() {
        let sessions = WireCodec.decodeSessionList(makeSessionListPayload())
        assertEqual(sessions.count, 1, "session list count")
        assertEqual(
            sessions[0],
            DecodedWireSessionInfo(
                id: 7,
                name: "Tab 1",
                title: "shell",
                pwd: "/tmp",
                attached: true,
                childExited: false
            ),
            "session list decoding"
        )

        guard let state = WireCodec.decodeFullState(makeFullStatePayload()) else {
            fputs("failed to decode full-state payload\n", stderr)
            exit(1)
        }
        assertEqual(state.rows, 1, "rows decode")
        assertEqual(state.cols, 2, "cols decode")
        assertEqual(state.cursorX, 1, "cursorX decode")
        assertEqual(state.cursorY, 0, "cursorY decode")
        assertEqual(state.cursorVisible, true, "cursor visible decode")
        assertEqual(state.cells.count, 2, "cell count decode")
        assertEqual(WireCodec.screenText(from: state), "A好", "screen text decoding")

        var deltaState = state
        let applied = WireCodec.applyDelta(makeDeltaPayload(), to: &deltaState)
        assertEqual(applied, true, "delta apply succeeds")
        assertEqual(deltaState.cursorX, 0, "delta cursorX decode")
        assertEqual(deltaState.cursorY, 0, "delta cursorY decode")
        assertEqual(deltaState.cursorVisible, true, "delta cursor visible decode")
        assertEqual(WireCodec.screenText(from: deltaState), "BC", "delta screen text decoding")

        var clientState = ClientWireState()
        let buildId = "0.1.0"
        var authOkPayload = Data(count: 8 + buildId.utf8.count)
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x0f).littleEndian, toByteOffset: 2, as: UInt32.self)
            bytes.storeBytes(of: UInt16(buildId.utf8.count).littleEndian, toByteOffset: 6, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(8..<(8 + buildId.utf8.count), with: buildId.utf8)
        let authEffect = ClientWireReducer.reduce(message: .authOk, payload: authOkPayload, state: &clientState)
        assertEqual(authEffect, .listSessions, "auth ok triggers session refresh")
        assertEqual(clientState.authenticated, true, "auth ok sets authenticated")
        assertEqual(clientState.protocolVersion, 1, "auth ok protocol version decode")
        assertEqual(clientState.transportCapabilities, 0x0f, "auth ok capability decode")
        assertEqual(clientState.serverBuildId, buildId, "auth ok build id decode")
        assertEqual(validateAuthOkMetadata(authOkPayload, authRequired: true), nil, "auth ok metadata validation")

        var missingBuildPayload = Data(count: 6)
        missingBuildPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x0f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingBuildPayload, authRequired: true),
            "Remote handshake is missing server build metadata",
            "missing build metadata rejected"
        )

        var wrongVersionPayload = authOkPayload
        wrongVersionPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(2).littleEndian, as: UInt16.self)
        }
        assertEqual(
            validateAuthOkMetadata(wrongVersionPayload, authRequired: true),
            "Unsupported remote protocol version: 2",
            "wrong protocol version rejected"
        )

        let listEffect = ClientWireReducer.reduce(message: .sessionList, payload: makeSessionListPayload(), state: &clientState)
        assertEqual(listEffect, .none, "session list has no side effect")
        assertEqual(clientState.sessions, sessions, "session list reducer decode")

        let createdPayload = UInt32(42).littleEndianBytes
        let createdEffect = ClientWireReducer.reduce(message: .sessionCreated, payload: Data(createdPayload), state: &clientState)
        assertEqual(createdEffect, .attach(42), "session created triggers attach")

        let attachedEffect = ClientWireReducer.reduce(message: .attached, payload: Data(createdPayload), state: &clientState)
        assertEqual(attachedEffect, .none, "attached has no side effect")
        assertEqual(clientState.attachedSessionId, 42, "attached stores session id")

        clientState.screen = state
        let deltaEffect = ClientWireReducer.reduce(message: .delta, payload: makeDeltaPayload(), state: &clientState)
        assertEqual(deltaEffect, .none, "delta has no side effect")
        assertEqual(clientState.screen.map(WireCodec.screenText(from:)), "BC", "delta reducer applies screen update")

        let detachedEffect = ClientWireReducer.reduce(message: .detached, payload: Data(), state: &clientState)
        assertEqual(detachedEffect, .none, "detached has no side effect")
        assertEqual(clientState.attachedSessionId, nil, "detached clears attached session")

        print("iOS wire codec self-test passed")
    }
}
