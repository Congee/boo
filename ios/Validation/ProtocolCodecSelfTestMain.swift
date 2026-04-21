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
        let serverInstanceId = "deadbeefcafebabe"
        let serverIdentityId = "daemon-identity-01"
        var authOkPayload = Data(count: 12 + buildId.utf8.count + serverInstanceId.utf8.count + serverIdentityId.utf8.count)
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x7f).littleEndian, toByteOffset: 2, as: UInt32.self)
            bytes.storeBytes(of: UInt16(buildId.utf8.count).littleEndian, toByteOffset: 6, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(8..<(8 + buildId.utf8.count), with: buildId.utf8)
        let instanceLengthOffset = 8 + buildId.utf8.count
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(serverInstanceId.utf8.count).littleEndian, toByteOffset: instanceLengthOffset, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(
            (instanceLengthOffset + 2)..<(instanceLengthOffset + 2 + serverInstanceId.utf8.count),
            with: serverInstanceId.utf8
        )
        let identityLengthOffset = instanceLengthOffset + 2 + serverInstanceId.utf8.count
        authOkPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(serverIdentityId.utf8.count).littleEndian, toByteOffset: identityLengthOffset, as: UInt16.self)
        }
        authOkPayload.replaceSubrange(
            (identityLengthOffset + 2)..<(identityLengthOffset + 2 + serverIdentityId.utf8.count),
            with: serverIdentityId.utf8
        )
        let authEffect = ClientWireReducer.reduce(message: .authOk, payload: authOkPayload, state: &clientState)
        assertEqual(authEffect, .listSessions, "auth ok triggers session refresh")
        assertEqual(clientState.authenticated, true, "auth ok sets authenticated")
        assertEqual(clientState.protocolVersion, 1, "auth ok protocol version decode")
        assertEqual(clientState.transportCapabilities, 0x7f, "auth ok capability decode")
        assertEqual(clientState.serverBuildId, buildId, "auth ok build id decode")
        assertEqual(clientState.serverInstanceId, serverInstanceId, "auth ok instance id decode")
        assertEqual(clientState.serverIdentityId, serverIdentityId, "auth ok identity id decode")
        assertEqual(validateAuthOkMetadata(authOkPayload), nil, "auth ok metadata validation")

        var missingBuildPayload = Data(count: 6)
        missingBuildPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(1).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt32(0x7f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingBuildPayload),
            "Remote handshake is missing server build metadata",
            "missing build metadata rejected"
        )

        var wrongVersionPayload = authOkPayload
        wrongVersionPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(2).littleEndian, as: UInt16.self)
        }
        assertEqual(
            validateAuthOkMetadata(wrongVersionPayload),
            "Unsupported remote protocol version: 2",
            "wrong protocol version rejected"
        )

        var missingHeartbeatPayload = authOkPayload
        missingHeartbeatPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(0x0f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingHeartbeatPayload),
            "Remote server does not advertise heartbeat support",
            "missing heartbeat capability rejected"
        )

        var missingResumePayload = authOkPayload
        missingResumePayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(0x1f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingResumePayload),
            "Remote server does not advertise attachment resume support",
            "missing attachment resume capability rejected"
        )

        var missingIdentityCapabilityPayload = authOkPayload
        missingIdentityCapabilityPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(0x3f).littleEndian, toByteOffset: 2, as: UInt32.self)
        }
        assertEqual(
            validateAuthOkMetadata(missingIdentityCapabilityPayload),
            "Remote server does not advertise daemon identity support",
            "missing daemon identity capability rejected"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: "daemon-a", actualIdentityId: "daemon-b"),
            true,
            "server identity mismatch detects changed daemon"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: "daemon-a", actualIdentityId: "daemon-a"),
            false,
            "server identity mismatch accepts same daemon"
        )
        assertEqual(
            serverIdentityMismatch(expectedIdentityId: nil, actualIdentityId: "daemon-a"),
            false,
            "server identity mismatch ignores unknown trusted identity"
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

        var attachedWithResumePayload = Data(count: 20)
        attachedWithResumePayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt32(42).littleEndian, as: UInt32.self)
            bytes.storeBytes(of: UInt64(0xB001D00DCAFEBEEF).littleEndian, toByteOffset: 4, as: UInt64.self)
            bytes.storeBytes(of: UInt64(0x0BADF00DDEADC0DE).littleEndian, toByteOffset: 12, as: UInt64.self)
        }
        let attachedWithResumeEffect = ClientWireReducer.reduce(
            message: .attached,
            payload: attachedWithResumePayload,
            state: &clientState
        )
        assertEqual(attachedWithResumeEffect, .none, "attached with resume token has no side effect")
        assertEqual(clientState.attachedSessionId, 42, "attached with resume token stores session id")
        assertEqual(clientState.attachmentId, 0xB001D00DCAFEBEEF, "attached with resume token stores attachment id")
        assertEqual(clientState.resumeToken, 0x0BADF00DDEADC0DE, "attached with resume token stores resume token")

        clientState.screen = state
        let deltaEffect = ClientWireReducer.reduce(message: .delta, payload: makeDeltaPayload(), state: &clientState)
        assertEqual(deltaEffect, .none, "delta has no side effect")
        assertEqual(clientState.screen.map(WireCodec.screenText(from:)), "BC", "delta reducer applies screen update")

        let detachedEffect = ClientWireReducer.reduce(message: .detached, payload: Data(), state: &clientState)
        assertEqual(detachedEffect, .none, "detached has no side effect")
        assertEqual(clientState.attachedSessionId, nil, "detached clears attached session")

        clientState.attachedSessionId = 42
        clientState.attachmentId = 0xB001D00DCAFEBEEF
        clientState.resumeToken = 0x0BADF00DDEADC0DE
        let sessionExitedEffect = ClientWireReducer.reduce(message: .sessionExited, payload: Data(), state: &clientState)
        assertEqual(sessionExitedEffect, .listSessions, "session exited triggers session refresh")
        assertEqual(clientState.attachedSessionId, nil, "session exited clears attached session")
        assertEqual(clientState.attachmentId, nil, "session exited clears attachment id")
        assertEqual(clientState.resumeToken, nil, "session exited clears resume token")

        let reachableSessions = [
            SessionInfo(
                id: 42,
                name: "Tab 1",
                title: "shell",
                pwd: "/tmp",
                attached: true,
                childExited: false
            )
        ]
        assertEqual(
            resolveAttachedSessionHealth(attachedSessionId: nil, sessions: reachableSessions),
            .unattached,
            "missing attachment is unhealthy"
        )
        assertEqual(
            resolveAttachedSessionHealth(attachedSessionId: 7, sessions: reachableSessions),
            .unreachable(sessionId: 7),
            "missing attached session is unreachable"
        )
        assertEqual(
            resolveAttachedSessionHealth(attachedSessionId: 42, sessions: reachableSessions),
            .reachable(sessionId: 42),
            "live attached session is reachable"
        )
        assertEqual(
            resolveAttachedSessionHealth(
                attachedSessionId: 9,
                sessions: [
                    SessionInfo(
                        id: 9,
                        name: "Tab 9",
                        title: "shell",
                        pwd: "/tmp",
                        attached: true,
                        childExited: true
                    )
                ]
            ),
            .exited(sessionId: 9),
            "exited session is not treated as reachable"
        )

        print("iOS wire codec self-test passed")
    }
}
